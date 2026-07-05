use std::sync::Arc;

use async_trait::async_trait;
use chrono::{Duration, Utc};
use deadpool_postgres::Pool;
use ironclaw_host_api::{ResourceScope, SecretHandle, Timestamp};
use secrecy::ExposeSecret;
use serde::{Deserialize, Serialize};

use crate::{
    DEFAULT_SECRET_LEASE_TTL_SECONDS, SecretError, SecretLease, SecretLeaseId, SecretLeaseStatus,
    SecretMaterial, SecretMetadata, SecretStore, SecretStoreError, SecretsCrypto,
    filesystem_secret_aad,
};

const SECRETS_TABLE: &str = "ironclaw_secret_records";
const LEASES_TABLE: &str = "ironclaw_secret_leases";

#[derive(Clone)]
pub struct PostgresSecretStore {
    pool: Pool,
    crypto: Arc<SecretsCrypto>,
    lease_ttl: Duration,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredSecret {
    scope: ResourceScope,
    handle: SecretHandle,
    encrypted_value: Vec<u8>,
    key_salt: Vec<u8>,
    expires_at: Option<Timestamp>,
    created_at: Timestamp,
    updated_at: Timestamp,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct StoredLease {
    scope: ResourceScope,
    handle: SecretHandle,
    lease_id: SecretLeaseId,
    status: SecretLeaseStatus,
    lease_expires_at: Timestamp,
    secret_expires_at: Option<Timestamp>,
}

impl PostgresSecretStore {
    pub fn new(pool: Pool, crypto: Arc<SecretsCrypto>) -> Self {
        Self {
            pool,
            crypto,
            lease_ttl: Duration::seconds(DEFAULT_SECRET_LEASE_TTL_SECONDS),
        }
    }

    pub fn with_lease_ttl(mut self, lease_ttl: Duration) -> Self {
        self.lease_ttl = lease_ttl;
        self
    }

    pub async fn run_migrations(&self) -> Result<(), SecretStoreError> {
        let client = self.connect().await?;
        client
            .batch_execute(
                r#"
                CREATE TABLE IF NOT EXISTS ironclaw_secret_records (
                    tenant_id TEXT NOT NULL,
                    user_id TEXT NOT NULL,
                    agent_id TEXT NOT NULL,
                    project_id TEXT NOT NULL,
                    handle TEXT NOT NULL,
                    record JSONB NOT NULL,
                    expires_at TEXT,
                    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                    PRIMARY KEY (tenant_id, user_id, agent_id, project_id, handle)
                );

                CREATE INDEX IF NOT EXISTS ironclaw_secret_records_owner_idx
                    ON ironclaw_secret_records (tenant_id, user_id, agent_id, project_id);

                CREATE TABLE IF NOT EXISTS ironclaw_secret_leases (
                    tenant_id TEXT NOT NULL,
                    user_id TEXT NOT NULL,
                    agent_id TEXT NOT NULL,
                    project_id TEXT NOT NULL,
                    mission_id TEXT NOT NULL,
                    thread_id TEXT NOT NULL,
                    invocation_id TEXT NOT NULL,
                    lease_id TEXT NOT NULL,
                    handle TEXT NOT NULL,
                    status TEXT NOT NULL,
                    record JSONB NOT NULL,
                    lease_expires_at TEXT NOT NULL,
                    secret_expires_at TEXT,
                    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                    PRIMARY KEY (
                        tenant_id,
                        user_id,
                        agent_id,
                        project_id,
                        mission_id,
                        thread_id,
                        invocation_id,
                        lease_id
                    )
                );

                CREATE INDEX IF NOT EXISTS ironclaw_secret_leases_owner_idx
                    ON ironclaw_secret_leases (
                        tenant_id,
                        user_id,
                        agent_id,
                        project_id,
                        mission_id,
                        thread_id,
                        invocation_id
                    );
                "#,
            )
            .await
            .map_err(|error| postgres_error("migrate secret store", error))?;
        Ok(())
    }

    async fn connect(&self) -> Result<deadpool_postgres::Object, SecretStoreError> {
        self.pool
            .get()
            .await
            .map_err(|error| SecretStoreError::StoreUnavailable {
                reason: format!("postgres secret store pool checkout failed: {error}"),
            })
    }

    async fn read_secret(
        &self,
        scope: &ResourceScope,
        handle: &SecretHandle,
    ) -> Result<Option<StoredSecret>, SecretStoreError> {
        let client = self.connect().await?;
        let row = client
            .query_opt(
                &format!(
                    "SELECT record FROM {SECRETS_TABLE}
                     WHERE tenant_id = $1
                       AND user_id = $2
                       AND agent_id = $3
                       AND project_id = $4
                       AND handle = $5"
                ),
                &[
                    &scope.tenant_id.as_str(),
                    &scope.user_id.as_str(),
                    &opt_key(scope.agent_id.as_ref().map(|id| id.as_str())),
                    &opt_key(scope.project_id.as_ref().map(|id| id.as_str())),
                    &handle.as_str(),
                ],
            )
            .await
            .map_err(|error| postgres_error("read secret", error))?;
        let Some(row) = row else {
            return Ok(None);
        };
        let stored = row_secret(&row)?;
        if !same_scope_owner(&stored.scope, scope) || &stored.handle != handle {
            return Ok(None);
        }
        Ok(Some(stored))
    }

    async fn read_lease_for_update(
        tx: &tokio_postgres::Transaction<'_>,
        scope: &ResourceScope,
        lease_id: SecretLeaseId,
    ) -> Result<Option<StoredLease>, SecretStoreError> {
        let row = tx
            .query_opt(
                &format!(
                    "SELECT record FROM {LEASES_TABLE}
                     WHERE tenant_id = $1
                       AND user_id = $2
                       AND agent_id = $3
                       AND project_id = $4
                       AND mission_id = $5
                       AND thread_id = $6
                       AND invocation_id = $7
                       AND lease_id = $8
                     FOR UPDATE"
                ),
                &[
                    &scope.tenant_id.as_str(),
                    &scope.user_id.as_str(),
                    &opt_key(scope.agent_id.as_ref().map(|id| id.as_str())),
                    &opt_key(scope.project_id.as_ref().map(|id| id.as_str())),
                    &opt_key(scope.mission_id.as_ref().map(|id| id.as_str())),
                    &opt_key(scope.thread_id.as_ref().map(|id| id.as_str())),
                    &scope.invocation_id.to_string(),
                    &lease_id.to_string(),
                ],
            )
            .await
            .map_err(|error| postgres_error("read secret lease", error))?;
        row.map(|row| row_lease(&row)).transpose()
    }

    async fn read_secret_in_tx(
        tx: &tokio_postgres::Transaction<'_>,
        scope: &ResourceScope,
        handle: &SecretHandle,
    ) -> Result<Option<StoredSecret>, SecretStoreError> {
        let row = tx
            .query_opt(
                &format!(
                    "SELECT record FROM {SECRETS_TABLE}
                     WHERE tenant_id = $1
                       AND user_id = $2
                       AND agent_id = $3
                       AND project_id = $4
                       AND handle = $5"
                ),
                &[
                    &scope.tenant_id.as_str(),
                    &scope.user_id.as_str(),
                    &opt_key(scope.agent_id.as_ref().map(|id| id.as_str())),
                    &opt_key(scope.project_id.as_ref().map(|id| id.as_str())),
                    &handle.as_str(),
                ],
            )
            .await
            .map_err(|error| postgres_error("read leased secret", error))?;
        row.map(|row| row_secret(&row)).transpose()
    }

    fn lease_to_public(stored: &StoredLease) -> SecretLease {
        SecretLease {
            id: stored.lease_id,
            scope: stored.scope.clone(),
            handle: stored.handle.clone(),
            status: stored.status,
        }
    }

    fn effective_status(stored: &StoredLease, now: Timestamp) -> SecretLeaseStatus {
        match stored.status {
            SecretLeaseStatus::Active => {
                let lease_expired = stored.lease_expires_at <= now;
                let secret_expired = stored
                    .secret_expires_at
                    .is_some_and(|expires_at| expires_at <= now);
                if lease_expired || secret_expired {
                    SecretLeaseStatus::Expired
                } else {
                    SecretLeaseStatus::Active
                }
            }
            other => other,
        }
    }
}

#[async_trait]
impl SecretStore for PostgresSecretStore {
    async fn put(
        &self,
        scope: ResourceScope,
        handle: SecretHandle,
        material: SecretMaterial,
        expires_at: Option<Timestamp>,
    ) -> Result<SecretMetadata, SecretStoreError> {
        let plaintext = material.expose_secret().as_bytes();
        let aad = filesystem_secret_aad(&scope, &handle);
        let (encrypted_value, key_salt) = self
            .crypto
            .encrypt(plaintext, &aad)
            .map_err(secret_error_to_store_error)?;
        let now = Utc::now();
        let stored = StoredSecret {
            scope: scope.clone(),
            handle: handle.clone(),
            encrypted_value,
            key_salt,
            expires_at,
            created_at: now,
            updated_at: now,
        };
        let record = serde_json::to_value(&stored).map_err(serde_to_store_error)?;
        let expires_at_text = expires_at.map(|value| value.to_rfc3339());
        let client = self.connect().await?;
        client
            .execute(
                &format!(
                    "INSERT INTO {SECRETS_TABLE}
                        (tenant_id, user_id, agent_id, project_id, handle, record, expires_at)
                     VALUES ($1, $2, $3, $4, $5, $6, $7)
                     ON CONFLICT (tenant_id, user_id, agent_id, project_id, handle)
                     DO UPDATE SET
                        record = EXCLUDED.record,
                        expires_at = EXCLUDED.expires_at,
                        updated_at = NOW()"
                ),
                &[
                    &scope.tenant_id.as_str(),
                    &scope.user_id.as_str(),
                    &opt_key(scope.agent_id.as_ref().map(|id| id.as_str())),
                    &opt_key(scope.project_id.as_ref().map(|id| id.as_str())),
                    &handle.as_str(),
                    &record,
                    &expires_at_text.as_deref(),
                ],
            )
            .await
            .map_err(|error| postgres_error("upsert secret", error))?;
        Ok(SecretMetadata {
            scope,
            handle,
            expires_at,
        })
    }

    async fn metadata(
        &self,
        scope: &ResourceScope,
        handle: &SecretHandle,
    ) -> Result<Option<SecretMetadata>, SecretStoreError> {
        Ok(self
            .read_secret(scope, handle)
            .await?
            .map(|stored| SecretMetadata {
                scope: stored.scope,
                handle: stored.handle,
                expires_at: stored.expires_at,
            }))
    }

    async fn metadata_for_scope(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<SecretMetadata>, SecretStoreError> {
        let client = self.connect().await?;
        let rows = client
            .query(
                &format!(
                    "SELECT record FROM {SECRETS_TABLE}
                     WHERE tenant_id = $1
                       AND user_id = $2
                       AND agent_id = $3
                       AND project_id = $4
                     ORDER BY handle"
                ),
                &[
                    &scope.tenant_id.as_str(),
                    &scope.user_id.as_str(),
                    &opt_key(scope.agent_id.as_ref().map(|id| id.as_str())),
                    &opt_key(scope.project_id.as_ref().map(|id| id.as_str())),
                ],
            )
            .await
            .map_err(|error| postgres_error("list secret metadata", error))?;
        rows.into_iter()
            .map(|row| {
                let stored = row_secret(&row)?;
                Ok(SecretMetadata {
                    scope: stored.scope,
                    handle: stored.handle,
                    expires_at: stored.expires_at,
                })
            })
            .collect()
    }

    async fn delete(
        &self,
        scope: &ResourceScope,
        handle: &SecretHandle,
    ) -> Result<bool, SecretStoreError> {
        let client = self.connect().await?;
        let deleted = client
            .execute(
                &format!(
                    "DELETE FROM {SECRETS_TABLE}
                     WHERE tenant_id = $1
                       AND user_id = $2
                       AND agent_id = $3
                       AND project_id = $4
                       AND handle = $5"
                ),
                &[
                    &scope.tenant_id.as_str(),
                    &scope.user_id.as_str(),
                    &opt_key(scope.agent_id.as_ref().map(|id| id.as_str())),
                    &opt_key(scope.project_id.as_ref().map(|id| id.as_str())),
                    &handle.as_str(),
                ],
            )
            .await
            .map_err(|error| postgres_error("delete secret", error))?;
        Ok(deleted > 0)
    }

    async fn lease_once(
        &self,
        scope: &ResourceScope,
        handle: &SecretHandle,
    ) -> Result<SecretLease, SecretStoreError> {
        let stored = self.read_secret(scope, handle).await?.ok_or_else(|| {
            SecretStoreError::UnknownSecret {
                scope: Box::new(scope.clone()),
                handle: handle.clone(),
            }
        })?;
        if let Some(expires_at) = stored.expires_at
            && expires_at <= Utc::now()
        {
            return Err(SecretStoreError::SecretExpired);
        }
        let lease_id = SecretLeaseId::new();
        let lease = StoredLease {
            scope: scope.clone(),
            handle: handle.clone(),
            lease_id,
            status: SecretLeaseStatus::Active,
            lease_expires_at: Utc::now() + self.lease_ttl,
            secret_expires_at: stored.expires_at,
        };
        let record = serde_json::to_value(&lease).map_err(serde_to_store_error)?;
        let lease_expires_at = lease.lease_expires_at.to_rfc3339();
        let secret_expires_at = lease.secret_expires_at.map(|value| value.to_rfc3339());
        let client = self.connect().await?;
        client
            .execute(
                &format!(
                    "INSERT INTO {LEASES_TABLE}
                        (
                            tenant_id,
                            user_id,
                            agent_id,
                            project_id,
                            mission_id,
                            thread_id,
                            invocation_id,
                            lease_id,
                            handle,
                            status,
                            record,
                            lease_expires_at,
                            secret_expires_at
                        )
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)"
                ),
                &[
                    &scope.tenant_id.as_str(),
                    &scope.user_id.as_str(),
                    &opt_key(scope.agent_id.as_ref().map(|id| id.as_str())),
                    &opt_key(scope.project_id.as_ref().map(|id| id.as_str())),
                    &opt_key(scope.mission_id.as_ref().map(|id| id.as_str())),
                    &opt_key(scope.thread_id.as_ref().map(|id| id.as_str())),
                    &scope.invocation_id.to_string(),
                    &lease_id.to_string(),
                    &handle.as_str(),
                    &lease_status_text(lease.status),
                    &record,
                    &lease_expires_at,
                    &secret_expires_at.as_deref(),
                ],
            )
            .await
            .map_err(|error| postgres_error("create secret lease", error))?;
        Ok(Self::lease_to_public(&lease))
    }

    async fn consume(
        &self,
        scope: &ResourceScope,
        lease_id: SecretLeaseId,
    ) -> Result<SecretMaterial, SecretStoreError> {
        let mut client = self.connect().await?;
        let tx = client
            .transaction()
            .await
            .map_err(|error| postgres_error("begin secret consume", error))?;
        let mut lease = Self::read_lease_for_update(&tx, scope, lease_id)
            .await?
            .ok_or_else(|| unknown_lease(scope, lease_id))?;
        if !same_scope_for_lease(&lease.scope, scope) {
            return Err(unknown_lease(scope, lease_id));
        }
        match Self::effective_status(&lease, Utc::now()) {
            SecretLeaseStatus::Consumed => Err(SecretStoreError::LeaseConsumed { lease_id }),
            SecretLeaseStatus::Revoked => Err(SecretStoreError::LeaseRevoked { lease_id }),
            SecretLeaseStatus::Expired => {
                if lease.status != SecretLeaseStatus::Expired {
                    lease.status = SecretLeaseStatus::Expired;
                    update_lease(&tx, &lease).await?;
                    tx.commit()
                        .await
                        .map_err(|error| postgres_error("commit expired secret lease", error))?;
                }
                Err(SecretStoreError::LeaseExpired { lease_id })
            }
            SecretLeaseStatus::Active => {
                let stored = Self::read_secret_in_tx(&tx, scope, &lease.handle)
                    .await?
                    .ok_or_else(|| SecretStoreError::UnknownSecret {
                        scope: Box::new(scope.clone()),
                        handle: lease.handle.clone(),
                    })?;
                let aad = filesystem_secret_aad(scope, &lease.handle);
                let decrypted = self
                    .crypto
                    .decrypt(&stored.encrypted_value, &stored.key_salt, &aad)
                    .map_err(secret_error_to_store_error)?;
                let material = SecretMaterial::from(decrypted.expose().to_string());
                lease.status = SecretLeaseStatus::Consumed;
                update_lease(&tx, &lease).await?;
                tx.commit()
                    .await
                    .map_err(|error| postgres_error("commit secret consume", error))?;
                Ok(material)
            }
        }
    }

    async fn revoke(
        &self,
        scope: &ResourceScope,
        lease_id: SecretLeaseId,
    ) -> Result<SecretLease, SecretStoreError> {
        let mut client = self.connect().await?;
        let tx = client
            .transaction()
            .await
            .map_err(|error| postgres_error("begin secret revoke", error))?;
        let mut lease = Self::read_lease_for_update(&tx, scope, lease_id)
            .await?
            .ok_or_else(|| unknown_lease(scope, lease_id))?;
        if !same_scope_for_lease(&lease.scope, scope) {
            return Err(unknown_lease(scope, lease_id));
        }
        if matches!(lease.status, SecretLeaseStatus::Active) {
            lease.status =
                if Self::effective_status(&lease, Utc::now()) == SecretLeaseStatus::Expired {
                    SecretLeaseStatus::Expired
                } else {
                    SecretLeaseStatus::Revoked
                };
            update_lease(&tx, &lease).await?;
        }
        tx.commit()
            .await
            .map_err(|error| postgres_error("commit secret revoke", error))?;
        Ok(Self::lease_to_public(&lease))
    }

    async fn leases_for_scope(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<SecretLease>, SecretStoreError> {
        let client = self.connect().await?;
        let rows = client
            .query(
                &format!(
                    "SELECT record FROM {LEASES_TABLE}
                     WHERE tenant_id = $1
                       AND user_id = $2
                       AND agent_id = $3
                       AND project_id = $4
                       AND mission_id = $5
                       AND thread_id = $6
                       AND invocation_id = $7
                     ORDER BY lease_id"
                ),
                &[
                    &scope.tenant_id.as_str(),
                    &scope.user_id.as_str(),
                    &opt_key(scope.agent_id.as_ref().map(|id| id.as_str())),
                    &opt_key(scope.project_id.as_ref().map(|id| id.as_str())),
                    &opt_key(scope.mission_id.as_ref().map(|id| id.as_str())),
                    &opt_key(scope.thread_id.as_ref().map(|id| id.as_str())),
                    &scope.invocation_id.to_string(),
                ],
            )
            .await
            .map_err(|error| postgres_error("list secret leases", error))?;
        rows.into_iter()
            .map(|row| row_lease(&row).map(|stored| Self::lease_to_public(&stored)))
            .collect()
    }
}

async fn update_lease(
    tx: &tokio_postgres::Transaction<'_>,
    lease: &StoredLease,
) -> Result<(), SecretStoreError> {
    let record = serde_json::to_value(lease).map_err(serde_to_store_error)?;
    tx.execute(
        &format!(
            "UPDATE {LEASES_TABLE}
             SET status = $1,
                 record = $2,
                 updated_at = NOW()
             WHERE tenant_id = $3
               AND user_id = $4
               AND agent_id = $5
               AND project_id = $6
               AND mission_id = $7
               AND thread_id = $8
               AND invocation_id = $9
               AND lease_id = $10"
        ),
        &[
            &lease_status_text(lease.status),
            &record,
            &lease.scope.tenant_id.as_str(),
            &lease.scope.user_id.as_str(),
            &opt_key(lease.scope.agent_id.as_ref().map(|id| id.as_str())),
            &opt_key(lease.scope.project_id.as_ref().map(|id| id.as_str())),
            &opt_key(lease.scope.mission_id.as_ref().map(|id| id.as_str())),
            &opt_key(lease.scope.thread_id.as_ref().map(|id| id.as_str())),
            &lease.scope.invocation_id.to_string(),
            &lease.lease_id.to_string(),
        ],
    )
    .await
    .map_err(|error| postgres_error("update secret lease", error))?;
    Ok(())
}

fn row_secret(row: &tokio_postgres::Row) -> Result<StoredSecret, SecretStoreError> {
    let record: serde_json::Value = row.get("record");
    serde_json::from_value(record).map_err(serde_to_store_error)
}

fn row_lease(row: &tokio_postgres::Row) -> Result<StoredLease, SecretStoreError> {
    let record: serde_json::Value = row.get("record");
    serde_json::from_value(record).map_err(serde_to_store_error)
}

fn opt_key(value: Option<&str>) -> String {
    value.unwrap_or("").to_string()
}

fn lease_status_text(status: SecretLeaseStatus) -> &'static str {
    match status {
        SecretLeaseStatus::Active => "active",
        SecretLeaseStatus::Consumed => "consumed",
        SecretLeaseStatus::Revoked => "revoked",
        SecretLeaseStatus::Expired => "expired",
    }
}

fn same_scope_owner(left: &ResourceScope, right: &ResourceScope) -> bool {
    left.tenant_id == right.tenant_id
        && left.user_id == right.user_id
        && left.agent_id == right.agent_id
        && left.project_id == right.project_id
}

fn same_scope_for_lease(left: &ResourceScope, right: &ResourceScope) -> bool {
    same_scope_owner(left, right)
        && left.mission_id == right.mission_id
        && left.thread_id == right.thread_id
        && left.invocation_id == right.invocation_id
}

fn unknown_lease(scope: &ResourceScope, lease_id: SecretLeaseId) -> SecretStoreError {
    SecretStoreError::UnknownLease {
        scope: Box::new(scope.clone()),
        lease_id,
    }
}

fn secret_error_to_store_error(error: SecretError) -> SecretStoreError {
    match error {
        SecretError::Expired => SecretStoreError::SecretExpired,
        SecretError::InvalidMasterKey => SecretStoreError::BackendMisconfigured {
            reason: error.to_string(),
        },
        other => SecretStoreError::StoreUnavailable {
            reason: other.to_string(),
        },
    }
}

fn serde_to_store_error(error: serde_json::Error) -> SecretStoreError {
    SecretStoreError::StoreUnavailable {
        reason: format!("failed to serialize postgres secret record: {error}"),
    }
}

fn postgres_error(operation: &'static str, error: tokio_postgres::Error) -> SecretStoreError {
    SecretStoreError::StoreUnavailable {
        reason: format!("postgres secret store {operation} failed: {error}"),
    }
}
