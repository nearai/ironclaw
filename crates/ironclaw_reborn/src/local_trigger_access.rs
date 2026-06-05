//! Local-dev trigger-fire access store.
//!
//! This is a Reborn-owned local bootstrap access store, separate from the
//! WebChat identity store. It runs on the same libSQL substrate file, but owns
//! only the local `local_reborn_access` table used to satisfy the fire-time
//! trigger authorization contract during local development.
//!
//! This table is not the production agent/project membership source of truth.
//! Production and multi-tenant runtimes must wire a real membership-backed
//! trigger access checker instead of this local bootstrap store.

use std::collections::BTreeSet;
use std::sync::Arc;

use chrono::{SecondsFormat, Utc};
use ironclaw_host_api::{AgentId, ProjectId, TenantId, UserId};
use thiserror::Error;

/// Fixed local-dev access role persisted on trigger-fire access rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalTriggerAccessRole {
    /// Owner-level local trigger-fire access.
    Owner,
}

impl LocalTriggerAccessRole {
    fn as_str(self) -> &'static str {
        match self {
            Self::Owner => "owner",
        }
    }
}

/// Local-dev bootstrap path that owns a trigger-fire access row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalTriggerAccessSource {
    /// Environment-token `serve` bootstrap path.
    LocalDevEnvBootstrap,
    /// SSO-admitted WebUI user bootstrap path.
    LocalDevSsoBootstrap,
    /// CLI `run` default-owner bootstrap path.
    LocalDevRunBootstrap,
}

impl LocalTriggerAccessSource {
    fn as_str(self) -> &'static str {
        match self {
            Self::LocalDevEnvBootstrap => "local_dev_env_bootstrap",
            Self::LocalDevSsoBootstrap => "local_dev_sso_bootstrap",
            Self::LocalDevRunBootstrap => "local_dev_run_bootstrap",
        }
    }
}

/// Fixed lifecycle state persisted on local-dev access rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LocalTriggerAccessStatus {
    Active,
    Inactive,
}

impl LocalTriggerAccessStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Inactive => "inactive",
        }
    }
}

/// Failure modes of the libSQL local trigger access store.
#[derive(Debug, Error)]
pub enum RebornLocalTriggerAccessStoreError {
    /// The libSQL backend (connect / migrate / query / commit) failed.
    #[error("reborn local trigger access store backend failure: {0}")]
    Backend(String),
}

/// Local-dev trigger access row to seed from trusted host/operator input.
pub struct LocalTriggerAccessSeed<'a> {
    /// Tenant scope for the local access row.
    pub tenant_id: &'a TenantId,
    /// User that is allowed to fire triggers for the exact scope.
    pub user_id: &'a UserId,
    /// Optional agent scope. `None` is stored as an exact no-agent scope, not
    /// a wildcard.
    pub agent_id: Option<&'a AgentId>,
    /// Optional project scope. `None` is stored as an exact no-project scope,
    /// not a wildcard.
    pub project_id: Option<&'a ProjectId>,
    /// Local role to persist on the access row.
    pub role: LocalTriggerAccessRole,
    /// Source for the host/operator seed path.
    pub source: LocalTriggerAccessSource,
}

/// Current trusted local-dev access set for one bootstrap source and exact
/// tenant/agent/project scope.
pub struct LocalTriggerAccessReconciliation<'a> {
    /// Tenant scope for the local access rows.
    pub tenant_id: &'a TenantId,
    /// Users that should keep active access for this bootstrap source/scope.
    pub user_ids: &'a [UserId],
    /// Optional agent scope. `None` is an exact no-agent scope, not a wildcard.
    pub agent_id: Option<&'a AgentId>,
    /// Optional project scope. `None` is an exact no-project scope, not a
    /// wildcard.
    pub project_id: Option<&'a ProjectId>,
    /// Local role for newly inserted rows.
    pub role: LocalTriggerAccessRole,
    /// Source for this host/operator seed path.
    pub source: LocalTriggerAccessSource,
}

/// libSQL-backed local-dev trigger access repository.
pub struct RebornLibSqlLocalTriggerAccessStore {
    db: Arc<libsql::Database>,
}

impl RebornLibSqlLocalTriggerAccessStore {
    /// Open the store on an existing libSQL substrate handle and run its
    /// idempotent migrations.
    pub async fn open(
        db: Arc<libsql::Database>,
    ) -> Result<Self, RebornLocalTriggerAccessStoreError> {
        let store = Self { db };
        store.run_migrations().await?;
        Ok(store)
    }

    /// A connection with a busy timeout set. This store shares the reborn
    /// substrate DB file with other local-dev stores, so contended writes must
    /// wait for the lock rather than fail immediately with `SQLITE_BUSY`.
    async fn conn(&self) -> Result<libsql::Connection, RebornLocalTriggerAccessStoreError> {
        let conn = self.db.connect().map_err(backend)?;
        conn.query("PRAGMA busy_timeout = 5000", ())
            .await
            .map_err(backend)?;
        Ok(conn)
    }

    async fn run_migrations(&self) -> Result<(), RebornLocalTriggerAccessStoreError> {
        let conn = self.conn().await?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS local_reborn_access (\
                 tenant_id TEXT NOT NULL, \
                 user_id TEXT NOT NULL, \
                 agent_id TEXT NOT NULL, \
                 project_id TEXT NOT NULL, \
                 role TEXT NOT NULL, \
                 status TEXT NOT NULL, \
                 source TEXT NOT NULL, \
                 created_at TEXT NOT NULL, \
                 updated_at TEXT NOT NULL, \
                 PRIMARY KEY (tenant_id, user_id, agent_id, project_id));",
        )
        .await
        .map_err(backend)?;
        Ok(())
    }

    /// Seed the local-dev trigger access row used by Reborn-owned fire-time
    /// trigger authorization. Existing rows are left untouched so a local
    /// operator can revoke or edit access without the next boot or login
    /// silently re-granting it.
    pub async fn seed_local_access(
        &self,
        seed: LocalTriggerAccessSeed<'_>,
    ) -> Result<(), RebornLocalTriggerAccessStoreError> {
        let conn = self.conn().await?;
        let tx = conn
            .transaction_with_behavior(libsql::TransactionBehavior::Immediate)
            .await
            .map_err(backend)?;
        let now = Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true);
        tx.execute(
            "INSERT INTO local_reborn_access \
                 (tenant_id, user_id, agent_id, project_id, role, status, source, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8) \
                 ON CONFLICT(tenant_id, user_id, agent_id, project_id) DO NOTHING",
            libsql::params![
                seed.tenant_id.as_str(),
                seed.user_id.as_str(),
                optional_scope_key(seed.agent_id.map(AgentId::as_str)),
                optional_scope_key(seed.project_id.map(ProjectId::as_str)),
                seed.role.as_str(),
                LocalTriggerAccessStatus::Active.as_str(),
                seed.source.as_str(),
                now.as_str(),
            ],
        )
        .await
        .map_err(backend)?;
        tx.commit().await.map_err(backend)?;
        Ok(())
    }

    /// Reconcile bootstrap-owned local-dev access rows for one exact scope.
    ///
    /// Active rows from the same `source` and scope that are not in
    /// `user_ids` are marked inactive, so local-dev boot/login admission
    /// changes stop authorizing stale creators. Existing inactive rows are
    /// still left untouched; marking a row inactive remains the local operator
    /// revocation mechanism and the next reconciliation will not silently
    /// reactivate it.
    pub async fn reconcile_local_access(
        &self,
        reconciliation: LocalTriggerAccessReconciliation<'_>,
    ) -> Result<(), RebornLocalTriggerAccessStoreError> {
        let conn = self.conn().await?;
        let tx = conn
            .transaction_with_behavior(libsql::TransactionBehavior::Immediate)
            .await
            .map_err(backend)?;
        let now = Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true);
        let agent_key = optional_scope_key(reconciliation.agent_id.map(AgentId::as_str));
        let project_key = optional_scope_key(reconciliation.project_id.map(ProjectId::as_str));
        let allowed: BTreeSet<&str> = reconciliation.user_ids.iter().map(UserId::as_str).collect();

        let mut rows = tx
            .query(
                "SELECT user_id \
                 FROM local_reborn_access \
                 WHERE tenant_id = ?1 \
                   AND agent_id = ?2 \
                   AND project_id = ?3 \
                   AND source = ?4 \
                   AND status = ?5",
                libsql::params![
                    reconciliation.tenant_id.as_str(),
                    agent_key,
                    project_key,
                    reconciliation.source.as_str(),
                    LocalTriggerAccessStatus::Active.as_str(),
                ],
            )
            .await
            .map_err(backend)?;
        let mut stale_user_ids = Vec::new();
        while let Some(row) = rows.next().await.map_err(backend)? {
            let user_id = row.get::<String>(0).map_err(backend)?;
            if !allowed.contains(user_id.as_str()) {
                stale_user_ids.push(user_id);
            }
        }
        drop(rows);

        for user_id in stale_user_ids {
            tx.execute(
                "UPDATE local_reborn_access \
                 SET status = ?1, updated_at = ?2 \
                 WHERE tenant_id = ?3 \
                   AND user_id = ?4 \
                   AND agent_id = ?5 \
                   AND project_id = ?6 \
                   AND source = ?7 \
                   AND status = ?8",
                libsql::params![
                    LocalTriggerAccessStatus::Inactive.as_str(),
                    now.as_str(),
                    reconciliation.tenant_id.as_str(),
                    user_id.as_str(),
                    agent_key,
                    project_key,
                    reconciliation.source.as_str(),
                    LocalTriggerAccessStatus::Active.as_str(),
                ],
            )
            .await
            .map_err(backend)?;
        }

        for user_id in reconciliation.user_ids {
            tx.execute(
                "INSERT INTO local_reborn_access \
                     (tenant_id, user_id, agent_id, project_id, role, status, source, created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8) \
                     ON CONFLICT(tenant_id, user_id, agent_id, project_id) DO NOTHING",
                libsql::params![
                    reconciliation.tenant_id.as_str(),
                    user_id.as_str(),
                    agent_key,
                    project_key,
                    reconciliation.role.as_str(),
                    LocalTriggerAccessStatus::Active.as_str(),
                    reconciliation.source.as_str(),
                    now.as_str(),
                ],
            )
            .await
            .map_err(backend)?;
        }

        tx.commit().await.map_err(backend)?;
        Ok(())
    }

    /// Return whether a local-dev user has active access for the exact
    /// tenant/agent/project tuple on a trigger fire request.
    pub async fn has_active_local_access(
        &self,
        tenant_id: &TenantId,
        user_id: &UserId,
        agent_id: Option<&AgentId>,
        project_id: Option<&ProjectId>,
    ) -> Result<bool, RebornLocalTriggerAccessStoreError> {
        let conn = self.conn().await?;
        let mut rows = conn
            .query(
                "SELECT 1 \
                 FROM local_reborn_access \
                 WHERE tenant_id = ?1 \
                   AND user_id = ?2 \
                   AND agent_id = ?3 \
                   AND project_id = ?4 \
                   AND status = ?5 \
                 LIMIT 1",
                libsql::params![
                    tenant_id.as_str(),
                    user_id.as_str(),
                    optional_scope_key(agent_id.map(AgentId::as_str)),
                    optional_scope_key(project_id.map(ProjectId::as_str)),
                    LocalTriggerAccessStatus::Active.as_str(),
                ],
            )
            .await
            .map_err(backend)?;
        Ok(rows.next().await.map_err(backend)?.is_some())
    }
}

fn backend(err: impl std::fmt::Display) -> RebornLocalTriggerAccessStoreError {
    RebornLocalTriggerAccessStoreError::Backend(err.to_string())
}

fn optional_scope_key(value: Option<&str>) -> &str {
    value.unwrap_or("")
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn store() -> RebornLibSqlLocalTriggerAccessStore {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.keep().join("reborn-local-dev.db");
        let db = Arc::new(
            libsql::Builder::new_local(&path)
                .build()
                .await
                .expect("open libsql"),
        );
        RebornLibSqlLocalTriggerAccessStore::open(db)
            .await
            .expect("open store")
    }

    #[tokio::test]
    async fn seeded_local_access_allows_exact_scope_only() {
        let store = store().await;
        let tenant_id = TenantId::new("local-access-tenant").expect("tenant id");
        let user_id = UserId::new("local-access-user").expect("user id");
        let other_user_id = UserId::new("local-access-other-user").expect("user id");
        let agent_id = AgentId::new("local-access-agent").expect("agent id");
        let project_id = ProjectId::new("local-access-project").expect("project id");
        let other_project_id = ProjectId::new("local-access-other-project").expect("project id");

        store
            .seed_local_access(LocalTriggerAccessSeed {
                tenant_id: &tenant_id,
                user_id: &user_id,
                agent_id: Some(&agent_id),
                project_id: Some(&project_id),
                role: LocalTriggerAccessRole::Owner,
                source: LocalTriggerAccessSource::LocalDevRunBootstrap,
            })
            .await
            .expect("seed local access");

        assert!(
            store
                .has_active_local_access(&tenant_id, &user_id, Some(&agent_id), Some(&project_id))
                .await
                .expect("check local access"),
            "the seeded exact tenant/user/agent/project scope is allowed"
        );
        assert!(
            !store
                .has_active_local_access(
                    &tenant_id,
                    &user_id,
                    Some(&agent_id),
                    Some(&other_project_id)
                )
                .await
                .expect("check local access"),
            "a different project is not covered by the seeded row"
        );
        assert!(
            !store
                .has_active_local_access(&tenant_id, &user_id, Some(&agent_id), None)
                .await
                .expect("check local access"),
            "a no-project request is not covered by a project-scoped row"
        );
        assert!(
            !store
                .has_active_local_access(
                    &tenant_id,
                    &other_user_id,
                    Some(&agent_id),
                    Some(&project_id)
                )
                .await
                .expect("check local access"),
            "a different user is not covered by the seeded row"
        );
    }

    #[tokio::test]
    async fn seeded_local_access_allows_exact_no_project_scope_only() {
        let store = store().await;
        let tenant_id = TenantId::new("local-no-project-tenant").expect("tenant id");
        let user_id = UserId::new("local-no-project-user").expect("user id");
        let agent_id = AgentId::new("local-no-project-agent").expect("agent id");
        let project_id = ProjectId::new("local-no-project-project").expect("project id");

        store
            .seed_local_access(LocalTriggerAccessSeed {
                tenant_id: &tenant_id,
                user_id: &user_id,
                agent_id: Some(&agent_id),
                project_id: None,
                role: LocalTriggerAccessRole::Owner,
                source: LocalTriggerAccessSource::LocalDevRunBootstrap,
            })
            .await
            .expect("seed local access");

        assert!(
            store
                .has_active_local_access(&tenant_id, &user_id, Some(&agent_id), None)
                .await
                .expect("check local access"),
            "the seeded no-project scope is allowed"
        );
        assert!(
            !store
                .has_active_local_access(&tenant_id, &user_id, Some(&agent_id), Some(&project_id))
                .await
                .expect("check local access"),
            "a no-project row is not a wildcard for project-scoped fires"
        );
    }

    #[tokio::test]
    async fn seed_local_access_does_not_reactivate_existing_inactive_row() {
        let store = store().await;
        let tenant_id = TenantId::new("local-revoked-tenant").expect("tenant id");
        let user_id = UserId::new("local-revoked-user").expect("user id");
        let agent_id = AgentId::new("local-revoked-agent").expect("agent id");
        let project_id = ProjectId::new("local-revoked-project").expect("project id");

        store
            .seed_local_access(LocalTriggerAccessSeed {
                tenant_id: &tenant_id,
                user_id: &user_id,
                agent_id: Some(&agent_id),
                project_id: Some(&project_id),
                role: LocalTriggerAccessRole::Owner,
                source: LocalTriggerAccessSource::LocalDevRunBootstrap,
            })
            .await
            .expect("seed local access");

        let conn = store.conn().await.expect("conn");
        conn.execute(
            "UPDATE local_reborn_access SET status = ?1 \
             WHERE tenant_id = ?2 AND user_id = ?3 AND agent_id = ?4 AND project_id = ?5",
            libsql::params![
                LocalTriggerAccessStatus::Inactive.as_str(),
                tenant_id.as_str(),
                user_id.as_str(),
                agent_id.as_str(),
                project_id.as_str(),
            ],
        )
        .await
        .expect("mark access inactive");

        store
            .seed_local_access(LocalTriggerAccessSeed {
                tenant_id: &tenant_id,
                user_id: &user_id,
                agent_id: Some(&agent_id),
                project_id: Some(&project_id),
                role: LocalTriggerAccessRole::Owner,
                source: LocalTriggerAccessSource::LocalDevRunBootstrap,
            })
            .await
            .expect("reseed local access");

        assert!(
            !store
                .has_active_local_access(&tenant_id, &user_id, Some(&agent_id), Some(&project_id))
                .await
                .expect("check local access"),
            "reseed must not silently reactivate an inactive existing row"
        );
    }

    #[tokio::test]
    async fn reconcile_local_access_deactivates_stale_source_rows_only() {
        let store = store().await;
        let tenant_id = TenantId::new("local-reconcile-tenant").expect("tenant id");
        let keep_user_id = UserId::new("local-reconcile-keep").expect("user id");
        let stale_user_id = UserId::new("local-reconcile-stale").expect("user id");
        let manual_user_id = UserId::new("local-reconcile-manual").expect("user id");
        let agent_id = AgentId::new("local-reconcile-agent").expect("agent id");
        let project_id = ProjectId::new("local-reconcile-project").expect("project id");

        for (user_id, source) in [
            (
                &keep_user_id,
                LocalTriggerAccessSource::LocalDevSsoBootstrap,
            ),
            (
                &stale_user_id,
                LocalTriggerAccessSource::LocalDevSsoBootstrap,
            ),
            (
                &manual_user_id,
                LocalTriggerAccessSource::LocalDevEnvBootstrap,
            ),
        ] {
            store
                .seed_local_access(LocalTriggerAccessSeed {
                    tenant_id: &tenant_id,
                    user_id,
                    agent_id: Some(&agent_id),
                    project_id: Some(&project_id),
                    role: LocalTriggerAccessRole::Owner,
                    source,
                })
                .await
                .expect("seed local access");
        }

        store
            .reconcile_local_access(LocalTriggerAccessReconciliation {
                tenant_id: &tenant_id,
                user_ids: std::slice::from_ref(&keep_user_id),
                agent_id: Some(&agent_id),
                project_id: Some(&project_id),
                role: LocalTriggerAccessRole::Owner,
                source: LocalTriggerAccessSource::LocalDevSsoBootstrap,
            })
            .await
            .expect("reconcile local access");

        assert!(
            store
                .has_active_local_access(
                    &tenant_id,
                    &keep_user_id,
                    Some(&agent_id),
                    Some(&project_id)
                )
                .await
                .expect("check local access"),
            "current bootstrap user remains active"
        );
        assert!(
            !store
                .has_active_local_access(
                    &tenant_id,
                    &stale_user_id,
                    Some(&agent_id),
                    Some(&project_id)
                )
                .await
                .expect("check local access"),
            "stale bootstrap user is deactivated"
        );
        assert!(
            store
                .has_active_local_access(
                    &tenant_id,
                    &manual_user_id,
                    Some(&agent_id),
                    Some(&project_id)
                )
                .await
                .expect("check local access"),
            "rows from another source are untouched"
        );
    }

    #[tokio::test]
    async fn reconcile_local_access_inserts_all_allowed_users() {
        let store = store().await;
        let tenant_id = TenantId::new("local-reconcile-many-tenant").expect("tenant id");
        let first_user_id = UserId::new("local-reconcile-many-first").expect("user id");
        let second_user_id = UserId::new("local-reconcile-many-second").expect("user id");
        let agent_id = AgentId::new("local-reconcile-many-agent").expect("agent id");
        let project_id = ProjectId::new("local-reconcile-many-project").expect("project id");
        let allowed_user_ids = [first_user_id.clone(), second_user_id.clone()];

        store
            .reconcile_local_access(LocalTriggerAccessReconciliation {
                tenant_id: &tenant_id,
                user_ids: &allowed_user_ids,
                agent_id: Some(&agent_id),
                project_id: Some(&project_id),
                role: LocalTriggerAccessRole::Owner,
                source: LocalTriggerAccessSource::LocalDevSsoBootstrap,
            })
            .await
            .expect("reconcile local access");

        for user_id in [&first_user_id, &second_user_id] {
            assert!(
                store
                    .has_active_local_access(
                        &tenant_id,
                        user_id,
                        Some(&agent_id),
                        Some(&project_id)
                    )
                    .await
                    .expect("check local access"),
                "every admitted user from one reconciliation should be inserted"
            );
        }
    }

    #[tokio::test]
    async fn reconcile_local_access_does_not_reactivate_inactive_allowed_user() {
        let store = store().await;
        let tenant_id = TenantId::new("local-reconcile-revoked-tenant").expect("tenant id");
        let user_id = UserId::new("local-reconcile-revoked-user").expect("user id");
        let agent_id = AgentId::new("local-reconcile-revoked-agent").expect("agent id");

        store
            .seed_local_access(LocalTriggerAccessSeed {
                tenant_id: &tenant_id,
                user_id: &user_id,
                agent_id: Some(&agent_id),
                project_id: None,
                role: LocalTriggerAccessRole::Owner,
                source: LocalTriggerAccessSource::LocalDevSsoBootstrap,
            })
            .await
            .expect("seed local access");

        let conn = store.conn().await.expect("conn");
        conn.execute(
            "UPDATE local_reborn_access SET status = ?1 \
             WHERE tenant_id = ?2 AND user_id = ?3 AND agent_id = ?4 AND project_id = ?5",
            libsql::params![
                LocalTriggerAccessStatus::Inactive.as_str(),
                tenant_id.as_str(),
                user_id.as_str(),
                agent_id.as_str(),
                optional_scope_key(None),
            ],
        )
        .await
        .expect("mark access inactive");

        store
            .reconcile_local_access(LocalTriggerAccessReconciliation {
                tenant_id: &tenant_id,
                user_ids: std::slice::from_ref(&user_id),
                agent_id: Some(&agent_id),
                project_id: None,
                role: LocalTriggerAccessRole::Owner,
                source: LocalTriggerAccessSource::LocalDevSsoBootstrap,
            })
            .await
            .expect("reconcile local access");

        assert!(
            !store
                .has_active_local_access(&tenant_id, &user_id, Some(&agent_id), None)
                .await
                .expect("check local access"),
            "reconcile must not silently reactivate an inactive existing row"
        );
    }
}
