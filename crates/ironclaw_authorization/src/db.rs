#[cfg(feature = "libsql")]
use std::sync::Arc;

use ironclaw_host_api::{
    CapabilityGrantId, ExecutionContext, InvocationFingerprint, ResourceScope,
};

use crate::{
    CapabilityLease, CapabilityLeaseError, CapabilityLeaseStatus, CapabilityLeaseStore,
    ensure_claimable, ensure_consumable, lease_is_authorizing, same_scope_owner,
};

#[cfg(feature = "libsql")]
const LIBSQL_LEASE_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS reborn_capability_lease_records (
    owner_key TEXT NOT NULL,
    invocation_id TEXT NOT NULL,
    lease_id TEXT NOT NULL,
    status TEXT NOT NULL,
    payload TEXT NOT NULL,
    PRIMARY KEY (owner_key, invocation_id, lease_id)
);
CREATE INDEX IF NOT EXISTS idx_reborn_capability_lease_records_owner
    ON reborn_capability_lease_records(owner_key, lease_id);
"#;

#[cfg(feature = "postgres")]
const POSTGRES_LEASE_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS reborn_capability_lease_records (
    owner_key TEXT NOT NULL,
    invocation_id TEXT NOT NULL,
    lease_id TEXT NOT NULL,
    status TEXT NOT NULL,
    payload JSONB NOT NULL,
    PRIMARY KEY (owner_key, invocation_id, lease_id)
);
CREATE INDEX IF NOT EXISTS idx_reborn_capability_lease_records_owner
    ON reborn_capability_lease_records(owner_key, lease_id);
"#;

#[cfg(feature = "libsql")]
pub struct LibSqlCapabilityLeaseStore {
    db: Arc<libsql::Database>,
}

#[cfg(feature = "libsql")]
impl LibSqlCapabilityLeaseStore {
    pub fn new(db: Arc<libsql::Database>) -> Self {
        Self { db }
    }

    pub async fn run_migrations(&self) -> Result<(), CapabilityLeaseError> {
        let conn = libsql_connect(&self.db).await?;
        conn.execute_batch(LIBSQL_LEASE_SCHEMA)
            .await
            .map_err(db_error)?;
        Ok(())
    }

    async fn connect(&self) -> Result<libsql::Connection, CapabilityLeaseError> {
        libsql_connect(&self.db).await
    }
}

#[cfg(feature = "libsql")]
#[async_trait::async_trait]
impl CapabilityLeaseStore for LibSqlCapabilityLeaseStore {
    async fn issue(&self, lease: CapabilityLease) -> Result<CapabilityLease, CapabilityLeaseError> {
        let conn = libsql_begin_immediate(&self.db).await?;
        let result = async {
            libsql_upsert_lease(&conn, &lease).await?;
            Ok(lease)
        }
        .await;
        finish_libsql_transaction(&conn, result).await
    }

    async fn revoke(
        &self,
        scope: &ResourceScope,
        lease_id: CapabilityGrantId,
    ) -> Result<CapabilityLease, CapabilityLeaseError> {
        self.update_lease(scope, lease_id, |lease| {
            lease.status = CapabilityLeaseStatus::Revoked;
            Ok(())
        })
        .await
    }

    async fn get(
        &self,
        scope: &ResourceScope,
        lease_id: CapabilityGrantId,
    ) -> Option<CapabilityLease> {
        let conn = self.connect().await.ok()?;
        libsql_get_lease(&conn, scope, lease_id)
            .await
            .ok()
            .flatten()
    }

    async fn claim(
        &self,
        scope: &ResourceScope,
        lease_id: CapabilityGrantId,
        invocation_fingerprint: &InvocationFingerprint,
    ) -> Result<CapabilityLease, CapabilityLeaseError> {
        self.update_lease(scope, lease_id, |lease| {
            ensure_claimable(lease, invocation_fingerprint)?;
            lease.status = CapabilityLeaseStatus::Claimed;
            Ok(())
        })
        .await
    }

    async fn consume(
        &self,
        scope: &ResourceScope,
        lease_id: CapabilityGrantId,
    ) -> Result<CapabilityLease, CapabilityLeaseError> {
        self.update_lease(scope, lease_id, consume_lease).await
    }

    async fn leases_for_scope(&self, scope: &ResourceScope) -> Vec<CapabilityLease> {
        let Ok(conn) = self.connect().await else {
            return Vec::new();
        };
        libsql_leases_for_scope(&conn, scope)
            .await
            .unwrap_or_default()
    }

    async fn active_leases_for_context(&self, context: &ExecutionContext) -> Vec<CapabilityLease> {
        self.leases_for_scope(&context.resource_scope)
            .await
            .into_iter()
            .filter(|lease| lease_is_authorizing(lease, context))
            .collect()
    }
}

#[cfg(feature = "libsql")]
impl LibSqlCapabilityLeaseStore {
    async fn update_lease<F>(
        &self,
        scope: &ResourceScope,
        lease_id: CapabilityGrantId,
        update: F,
    ) -> Result<CapabilityLease, CapabilityLeaseError>
    where
        F: FnOnce(&mut CapabilityLease) -> Result<(), CapabilityLeaseError>,
    {
        let conn = libsql_begin_immediate(&self.db).await?;
        let result = async {
            let mut lease = libsql_get_lease(&conn, scope, lease_id)
                .await?
                .ok_or(CapabilityLeaseError::UnknownLease { lease_id })?;
            update(&mut lease)?;
            libsql_upsert_lease(&conn, &lease).await?;
            Ok(lease)
        }
        .await;
        finish_libsql_transaction(&conn, result).await
    }
}

#[cfg(feature = "postgres")]
pub struct PostgresCapabilityLeaseStore {
    pool: deadpool_postgres::Pool,
}

#[cfg(feature = "postgres")]
impl PostgresCapabilityLeaseStore {
    pub fn new(pool: deadpool_postgres::Pool) -> Self {
        Self { pool }
    }

    pub async fn run_migrations(&self) -> Result<(), CapabilityLeaseError> {
        let client = self.pool.get().await.map_err(db_error)?;
        client
            .batch_execute(POSTGRES_LEASE_SCHEMA)
            .await
            .map_err(db_error)
    }
}

#[cfg(feature = "postgres")]
#[async_trait::async_trait]
impl CapabilityLeaseStore for PostgresCapabilityLeaseStore {
    async fn issue(&self, lease: CapabilityLease) -> Result<CapabilityLease, CapabilityLeaseError> {
        let client = self.pool.get().await.map_err(db_error)?;
        postgres_upsert_lease(&client, &lease).await?;
        Ok(lease)
    }

    async fn revoke(
        &self,
        scope: &ResourceScope,
        lease_id: CapabilityGrantId,
    ) -> Result<CapabilityLease, CapabilityLeaseError> {
        self.update_lease(scope, lease_id, |lease| {
            lease.status = CapabilityLeaseStatus::Revoked;
            Ok(())
        })
        .await
    }

    async fn get(
        &self,
        scope: &ResourceScope,
        lease_id: CapabilityGrantId,
    ) -> Option<CapabilityLease> {
        let client = self.pool.get().await.ok()?;
        postgres_get_lease(&client, scope, lease_id, false)
            .await
            .ok()
            .flatten()
    }

    async fn claim(
        &self,
        scope: &ResourceScope,
        lease_id: CapabilityGrantId,
        invocation_fingerprint: &InvocationFingerprint,
    ) -> Result<CapabilityLease, CapabilityLeaseError> {
        self.update_lease(scope, lease_id, |lease| {
            ensure_claimable(lease, invocation_fingerprint)?;
            lease.status = CapabilityLeaseStatus::Claimed;
            Ok(())
        })
        .await
    }

    async fn consume(
        &self,
        scope: &ResourceScope,
        lease_id: CapabilityGrantId,
    ) -> Result<CapabilityLease, CapabilityLeaseError> {
        self.update_lease(scope, lease_id, consume_lease).await
    }

    async fn leases_for_scope(&self, scope: &ResourceScope) -> Vec<CapabilityLease> {
        let Ok(client) = self.pool.get().await else {
            return Vec::new();
        };
        postgres_leases_for_scope(&client, scope)
            .await
            .unwrap_or_default()
    }

    async fn active_leases_for_context(&self, context: &ExecutionContext) -> Vec<CapabilityLease> {
        self.leases_for_scope(&context.resource_scope)
            .await
            .into_iter()
            .filter(|lease| lease_is_authorizing(lease, context))
            .collect()
    }
}

#[cfg(feature = "postgres")]
impl PostgresCapabilityLeaseStore {
    async fn update_lease<F>(
        &self,
        scope: &ResourceScope,
        lease_id: CapabilityGrantId,
        update: F,
    ) -> Result<CapabilityLease, CapabilityLeaseError>
    where
        F: FnOnce(&mut CapabilityLease) -> Result<(), CapabilityLeaseError>,
    {
        let mut client = self.pool.get().await.map_err(db_error)?;
        let transaction = client.transaction().await.map_err(db_error)?;
        let result = async {
            let mut lease = postgres_get_lease(&transaction, scope, lease_id, true)
                .await?
                .ok_or(CapabilityLeaseError::UnknownLease { lease_id })?;
            update(&mut lease)?;
            postgres_upsert_lease(&transaction, &lease).await?;
            Ok(lease)
        }
        .await;
        match result {
            Ok(lease) => {
                transaction.commit().await.map_err(db_error)?;
                Ok(lease)
            }
            Err(error) => {
                let _ = transaction.rollback().await;
                Err(error)
            }
        }
    }
}

fn consume_lease(lease: &mut CapabilityLease) -> Result<(), CapabilityLeaseError> {
    let was_claimed = lease.status == CapabilityLeaseStatus::Claimed;
    ensure_consumable(lease)?;
    if lease.invocation_fingerprint.is_some() {
        if let Some(remaining) = lease.grant.constraints.max_invocations.as_mut() {
            *remaining = 0;
        }
        lease.status = CapabilityLeaseStatus::Consumed;
    } else if let Some(remaining) = lease.grant.constraints.max_invocations.as_mut() {
        *remaining -= 1;
        if *remaining == 0 {
            lease.status = CapabilityLeaseStatus::Consumed;
        } else if was_claimed {
            lease.status = CapabilityLeaseStatus::Active;
        }
    } else if was_claimed {
        lease.status = CapabilityLeaseStatus::Active;
    }
    Ok(())
}

#[cfg(feature = "libsql")]
async fn libsql_connect(db: &libsql::Database) -> Result<libsql::Connection, CapabilityLeaseError> {
    let conn = db.connect().map_err(db_error)?;
    conn.query("PRAGMA busy_timeout = 5000", ())
        .await
        .map_err(db_error)?;
    Ok(conn)
}

#[cfg(feature = "libsql")]
async fn libsql_begin_immediate(
    db: &libsql::Database,
) -> Result<libsql::Connection, CapabilityLeaseError> {
    let conn = libsql_connect(db).await?;
    conn.execute("BEGIN IMMEDIATE", ())
        .await
        .map_err(db_error)?;
    Ok(conn)
}

#[cfg(feature = "libsql")]
async fn finish_libsql_transaction<T>(
    conn: &libsql::Connection,
    result: Result<T, CapabilityLeaseError>,
) -> Result<T, CapabilityLeaseError> {
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
async fn libsql_get_lease(
    conn: &libsql::Connection,
    scope: &ResourceScope,
    lease_id: CapabilityGrantId,
) -> Result<Option<CapabilityLease>, CapabilityLeaseError> {
    let mut rows = conn.query("SELECT invocation_id, status, payload FROM reborn_capability_lease_records WHERE owner_key = ?1 AND invocation_id = ?2 AND lease_id = ?3", libsql::params![owner_key(scope)?, scope.invocation_id.to_string(), lease_id.to_string()]).await.map_err(db_error)?;
    let Some(row) = rows.next().await.map_err(db_error)? else {
        return Ok(None);
    };
    let invocation_id: String = row.get(0).map_err(db_error)?;
    let status: String = row.get(1).map_err(db_error)?;
    let payload: String = row.get(2).map_err(db_error)?;
    validate_lease_row(
        from_json(&payload)?,
        scope,
        lease_id,
        &invocation_id,
        &status,
    )
    .map(Some)
}

#[cfg(feature = "libsql")]
async fn libsql_upsert_lease(
    conn: &libsql::Connection,
    lease: &CapabilityLease,
) -> Result<(), CapabilityLeaseError> {
    conn.execute("INSERT INTO reborn_capability_lease_records (owner_key, invocation_id, lease_id, status, payload) VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT(owner_key, invocation_id, lease_id) DO UPDATE SET status = excluded.status, payload = excluded.payload", libsql::params![owner_key(&lease.scope)?, lease.scope.invocation_id.to_string(), lease.grant.id.to_string(), lease_status_key(lease.status), to_json(lease)?]).await.map_err(db_error)?;
    Ok(())
}

#[cfg(feature = "libsql")]
async fn libsql_leases_for_scope(
    conn: &libsql::Connection,
    scope: &ResourceScope,
) -> Result<Vec<CapabilityLease>, CapabilityLeaseError> {
    let mut rows = conn.query("SELECT invocation_id, lease_id, status, payload FROM reborn_capability_lease_records WHERE owner_key = ?1 ORDER BY lease_id", libsql::params![owner_key(scope)?]).await.map_err(db_error)?;
    let mut leases = Vec::new();
    while let Some(row) = rows.next().await.map_err(db_error)? {
        let invocation_id: String = row.get(0).map_err(db_error)?;
        let lease_id: String = row.get(1).map_err(db_error)?;
        let lease_id = parse_lease_id(&lease_id)?;
        let status: String = row.get(2).map_err(db_error)?;
        let payload: String = row.get(3).map_err(db_error)?;
        leases.push(validate_lease_row(
            from_json(&payload)?,
            scope,
            lease_id,
            &invocation_id,
            &status,
        )?);
    }
    leases.sort_by_key(|lease| lease.grant.id.as_uuid());
    Ok(leases)
}

#[cfg(feature = "postgres")]
async fn postgres_get_lease(
    client: &impl deadpool_postgres::GenericClient,
    scope: &ResourceScope,
    lease_id: CapabilityGrantId,
    for_update: bool,
) -> Result<Option<CapabilityLease>, CapabilityLeaseError> {
    let query = if for_update {
        "SELECT invocation_id, status, payload::text FROM reborn_capability_lease_records WHERE owner_key = $1 AND invocation_id = $2 AND lease_id = $3 FOR UPDATE"
    } else {
        "SELECT invocation_id, status, payload::text FROM reborn_capability_lease_records WHERE owner_key = $1 AND invocation_id = $2 AND lease_id = $3"
    };
    let row = client
        .query_opt(
            query,
            &[
                &owner_key(scope)?,
                &scope.invocation_id.to_string(),
                &lease_id.to_string(),
            ],
        )
        .await
        .map_err(db_error)?;
    let Some(row) = row else {
        return Ok(None);
    };
    let invocation_id: String = row.get(0);
    let status: String = row.get(1);
    let payload: String = row.get(2);
    validate_lease_row(
        from_json(&payload)?,
        scope,
        lease_id,
        &invocation_id,
        &status,
    )
    .map(Some)
}

#[cfg(feature = "postgres")]
async fn postgres_upsert_lease(
    client: &impl deadpool_postgres::GenericClient,
    lease: &CapabilityLease,
) -> Result<(), CapabilityLeaseError> {
    client.execute("INSERT INTO reborn_capability_lease_records (owner_key, invocation_id, lease_id, status, payload) VALUES ($1, $2, $3, $4, $5::jsonb) ON CONFLICT(owner_key, invocation_id, lease_id) DO UPDATE SET status = EXCLUDED.status, payload = EXCLUDED.payload", &[&owner_key(&lease.scope)?, &lease.scope.invocation_id.to_string(), &lease.grant.id.to_string(), &lease_status_key(lease.status), &to_json(lease)?]).await.map_err(db_error)?;
    Ok(())
}

#[cfg(feature = "postgres")]
async fn postgres_leases_for_scope(
    client: &impl deadpool_postgres::GenericClient,
    scope: &ResourceScope,
) -> Result<Vec<CapabilityLease>, CapabilityLeaseError> {
    let rows = client.query("SELECT invocation_id, lease_id, status, payload::text FROM reborn_capability_lease_records WHERE owner_key = $1 ORDER BY lease_id", &[&owner_key(scope)?]).await.map_err(db_error)?;
    let mut leases = Vec::new();
    for row in rows {
        let invocation_id: String = row.get(0);
        let lease_id: String = row.get(1);
        let lease_id = parse_lease_id(&lease_id)?;
        let status: String = row.get(2);
        let payload: String = row.get(3);
        leases.push(validate_lease_row(
            from_json(&payload)?,
            scope,
            lease_id,
            &invocation_id,
            &status,
        )?);
    }
    leases.sort_by_key(|lease| lease.grant.id.as_uuid());
    Ok(leases)
}

fn validate_lease_row(
    lease: CapabilityLease,
    expected_scope: &ResourceScope,
    expected_lease_id: CapabilityGrantId,
    row_invocation_id: &str,
    row_status: &str,
) -> Result<CapabilityLease, CapabilityLeaseError> {
    if !same_scope_owner(&lease.scope, expected_scope)
        || lease.scope.invocation_id.to_string() != row_invocation_id
        || lease.grant.id != expected_lease_id
        || lease_status_key(lease.status) != row_status
    {
        return Err(CapabilityLeaseError::Persistence {
            reason: "capability lease row payload mismatch".to_string(),
        });
    }
    Ok(lease)
}

fn owner_key(scope: &ResourceScope) -> Result<String, CapabilityLeaseError> {
    #[derive(serde::Serialize)]
    struct OwnerKey<'a> {
        tenant_id: &'a str,
        user_id: &'a str,
        agent_id: Option<&'a str>,
        project_id: Option<&'a str>,
        mission_id: Option<&'a str>,
        thread_id: Option<&'a str>,
    }
    to_json(&OwnerKey {
        tenant_id: scope.tenant_id.as_str(),
        user_id: scope.user_id.as_str(),
        agent_id: scope.agent_id.as_ref().map(|id| id.as_str()),
        project_id: scope.project_id.as_ref().map(|id| id.as_str()),
        mission_id: scope.mission_id.as_ref().map(|id| id.as_str()),
        thread_id: scope.thread_id.as_ref().map(|id| id.as_str()),
    })
}

fn lease_status_key(status: CapabilityLeaseStatus) -> &'static str {
    match status {
        CapabilityLeaseStatus::Active => "active",
        CapabilityLeaseStatus::Claimed => "claimed",
        CapabilityLeaseStatus::Consumed => "consumed",
        CapabilityLeaseStatus::Revoked => "revoked",
    }
}

fn parse_lease_id(value: &str) -> Result<CapabilityGrantId, CapabilityLeaseError> {
    CapabilityGrantId::parse(value).map_err(|error| CapabilityLeaseError::Persistence {
        reason: error.to_string(),
    })
}

fn to_json<T: serde::Serialize>(value: &T) -> Result<String, CapabilityLeaseError> {
    serde_json::to_string(value).map_err(|error| CapabilityLeaseError::Persistence {
        reason: error.to_string(),
    })
}

fn from_json<T: serde::de::DeserializeOwned>(payload: &str) -> Result<T, CapabilityLeaseError> {
    serde_json::from_str(payload).map_err(|error| CapabilityLeaseError::Persistence {
        reason: error.to_string(),
    })
}

fn db_error(error: impl std::fmt::Display) -> CapabilityLeaseError {
    tracing::debug!(%error, "capability lease database operation failed");
    CapabilityLeaseError::Persistence {
        reason: "capability lease database unavailable".to_string(),
    }
}
