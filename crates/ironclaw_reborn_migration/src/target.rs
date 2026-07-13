//! Reborn write side.
//!
//! Opens the Reborn `RootFilesystem` substrate (and, for triggers, the raw
//! backend DB handle), builds every per-domain write service, and hands them to
//! the converters. Threads / secrets / identity force a concrete filesystem
//! type, so they are constructed inside the backend match arm where `F` is
//! known, then stored as `#[async_trait]` trait objects so the converters stay
//! backend-agnostic. All state is written under one (tenant, agent) scope from
//! [`MigrationOptions`]; each v1 `user_id` becomes the per-record Reborn `UserId`.

use std::sync::Arc;

use ironclaw_extensions::ExtensionInstallationStore;
use ironclaw_filesystem::{RootFilesystem, ScopedFilesystem};
use ironclaw_host_api::{AgentId, ProjectId, TenantId, UserId};
use ironclaw_memory::MemoryService;
use ironclaw_memory_native::NativeMemoryService;
use ironclaw_projects::{FilesystemProjectRepository, ProjectRepository};
use ironclaw_reborn_identity::{
    FilesystemRebornIdentityStore, RebornIdentityResolver, RebornUserDirectory,
};
use ironclaw_secrets::{FilesystemSecretStore, SecretStore, SecretsCrypto};
use ironclaw_threads::{FilesystemSessionThreadService, SessionThreadService};
use ironclaw_triggers::TriggerRepository;
use secrecy::SecretString;

use crate::error::MigrationError;
use crate::mounts;
use crate::options::{MigrationOptions, TargetStore};

#[path = "target_ids.rs"]
pub(crate) mod ids;

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct TargetReadback {
    pub(crate) users: u64,
    pub(crate) threads: u64,
    pub(crate) messages: u64,
    pub(crate) projects: u64,
    pub(crate) triggers: u64,
    pub(crate) memory_documents: u64,
    pub(crate) secrets: u64,
    pub(crate) identity_records: u64,
}

/// Inspect whether a target contains live Reborn state without applying schema
/// migrations or creating any target object.
pub(crate) async fn target_is_empty(target: &TargetStore) -> Result<bool, MigrationError> {
    match target {
        TargetStore::LibSql { path } => Ok(!path.exists()),
        #[cfg(feature = "postgres")]
        TargetStore::Postgres { url } => {
            let pool = open_postgres_pool(url)?;
            let client = pool.get().await.map_err(|error| {
                MigrationError::OpenTarget(format!(
                    "PostgreSQL target emptiness probe failed (details redacted): {}",
                    error
                ))
            })?;
            let relations = client
                .query(
                    "SELECT format('%I.%I', namespace.nspname, relation.relname)
                     FROM pg_catalog.pg_class relation
                     JOIN pg_catalog.pg_namespace namespace
                       ON namespace.oid = relation.relnamespace
                     WHERE relation.relkind IN ('r', 'p')
                       AND namespace.nspname NOT LIKE 'pg\\_%' ESCAPE '\\'
                       AND namespace.nspname <> 'information_schema'",
                    &[],
                )
                .await
                .map_err(|error| {
                    MigrationError::OpenTarget(format!(
                        "PostgreSQL target schema inventory failed: {error}"
                    ))
                })?;
            for relation in relations {
                let table: String = relation
                    .try_get(0)
                    .map_err(|error| MigrationError::OpenTarget(error.to_string()))?;
                let sql = format!("SELECT EXISTS (SELECT 1 FROM {table} LIMIT 1)");
                let populated: bool = client
                    .query_one(&sql, &[])
                    .await
                    .map_err(|error| {
                        MigrationError::OpenTarget(format!(
                            "PostgreSQL target data probe failed for {table}: {error}"
                        ))
                    })?
                    .try_get(0)
                    .map_err(|error| MigrationError::OpenTarget(error.to_string()))?;
                if populated {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        #[cfg(not(feature = "postgres"))]
        TargetStore::Postgres { .. } => Err(MigrationError::OpenTarget(
            "binary built without the postgres feature".to_string(),
        )),
    }
}

pub(crate) async fn write_shared_migration_state(
    target: &TargetStore,
    manifest: &crate::manifest::MigrationManifest,
) -> Result<(), MigrationError> {
    match target {
        TargetStore::LibSql { .. } => {
            let _ = manifest;
            Ok(())
        }
        #[cfg(feature = "postgres")]
        TargetStore::Postgres { url } => {
            let pool = open_postgres_pool(url)?;
            let mut client = pool.get().await.map_err(postgres_identity_error)?;
            client
                .batch_execute(
                    "CREATE TABLE IF NOT EXISTS reborn_migration_state (
                        singleton BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (singleton),
                        schema_version TEXT NOT NULL,
                        migration_protocol_version BIGINT NOT NULL,
                        release_version TEXT NOT NULL,
                        run_id TEXT NOT NULL,
                        status TEXT NOT NULL,
                        profile TEXT NOT NULL,
                        updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
                    )",
                )
                .await
                .map_err(postgres_identity_error)?;
            let transaction = client
                .transaction()
                .await
                .map_err(postgres_identity_error)?;
            let schema_version = "ironclaw.reborn.migration-state/v1";
            let protocol_version = i64::from(crate::manifest::MIGRATION_PROTOCOL_VERSION);
            let run_id = manifest.run_id.to_string();
            let status = migration_status_label(manifest.status);
            let existing = transaction
                .query_opt(
                    "SELECT schema_version, migration_protocol_version, release_version,
                            run_id, status, profile
                     FROM reborn_migration_state
                     WHERE singleton = TRUE
                     FOR UPDATE",
                    &[],
                )
                .await
                .map_err(postgres_identity_error)?;
            if let Some(existing) = existing {
                let existing_schema: String =
                    existing.try_get(0).map_err(postgres_identity_error)?;
                let existing_protocol: i64 =
                    existing.try_get(1).map_err(postgres_identity_error)?;
                let existing_release: String =
                    existing.try_get(2).map_err(postgres_identity_error)?;
                let existing_run_id: String =
                    existing.try_get(3).map_err(postgres_identity_error)?;
                let existing_status: String =
                    existing.try_get(4).map_err(postgres_identity_error)?;
                let existing_profile: String =
                    existing.try_get(5).map_err(postgres_identity_error)?;
                if existing_schema != schema_version
                    || existing_protocol != protocol_version
                    || existing_release != manifest.release_version
                    || existing_run_id != run_id
                    || existing_profile != manifest.scope.profile
                {
                    return Err(MigrationError::OpenTarget(
                        "PostgreSQL target is claimed by a different migration run or protocol"
                            .to_string(),
                    ));
                }
                if !shared_state_transition_allowed(&existing_status, manifest.status) {
                    return Err(MigrationError::OpenTarget(format!(
                        "invalid shared migration state transition {existing_status} -> {status}"
                    )));
                }
                transaction
                    .execute(
                        "UPDATE reborn_migration_state
                         SET status = $1, updated_at = NOW()
                         WHERE singleton = TRUE AND run_id = $2",
                        &[&status, &run_id],
                    )
                    .await
                    .map_err(postgres_identity_error)?;
            } else {
                if manifest.status != crate::manifest::MigrationStatus::Applying {
                    return Err(MigrationError::OpenTarget(
                        "PostgreSQL target has no active migration claim".to_string(),
                    ));
                }
                let parameters: [&(dyn tokio_postgres::types::ToSql + Sync); 6] = [
                    &schema_version,
                    &protocol_version,
                    &manifest.release_version,
                    &run_id,
                    &status,
                    &manifest.scope.profile,
                ];
                transaction
                    .execute(
                        "INSERT INTO reborn_migration_state (
                        singleton, schema_version, migration_protocol_version,
                        release_version, run_id, status, profile, updated_at
                     ) VALUES (TRUE, $1, $2, $3, $4, $5, $6, NOW())",
                        &parameters,
                    )
                    .await
                    .map_err(postgres_identity_error)?;
            }
            transaction
                .commit()
                .await
                .map_err(postgres_identity_error)?;
            Ok(())
        }
        #[cfg(not(feature = "postgres"))]
        TargetStore::Postgres { .. } => Err(MigrationError::OpenTarget(
            "binary built without the postgres feature".to_string(),
        )),
    }
}

#[cfg(feature = "postgres")]
const fn migration_status_label(status: crate::manifest::MigrationStatus) -> &'static str {
    use crate::manifest::MigrationStatus;
    match status {
        MigrationStatus::Planned => "planned",
        MigrationStatus::Applying => "applying",
        MigrationStatus::Failed => "failed",
        MigrationStatus::Applied => "applied",
        MigrationStatus::Verifying => "verifying",
        MigrationStatus::Verified => "verified",
    }
}

#[cfg(feature = "postgres")]
fn shared_state_transition_allowed(current: &str, next: crate::manifest::MigrationStatus) -> bool {
    use crate::manifest::MigrationStatus;

    current == migration_status_label(next)
        || matches!(
            (current, next),
            (
                "applying",
                MigrationStatus::Failed | MigrationStatus::Applied
            ) | ("failed", MigrationStatus::Applying)
                | (
                    "applied",
                    MigrationStatus::Applying | MigrationStatus::Verifying
                )
                | (
                    "verifying",
                    MigrationStatus::Failed | MigrationStatus::Verified
                )
                | ("verified", MigrationStatus::Verifying)
        )
}

/// Read migrated state through the same durable tables used by production,
/// without running migrations or starting workers/ingress.
pub(crate) async fn readback(
    target: &TargetStore,
    tenant_id: &TenantId,
) -> Result<TargetReadback, MigrationError> {
    let tenant = tenant_id.as_str();
    let thread_pattern = format!("/tenants/{tenant}/users/%/threads/%/thread.json");
    let message_pattern = format!("/tenants/{tenant}/users/%/threads/%/messages/%.json");
    let append_pattern = format!("/tenants/{tenant}/users/%/threads/%/message_appends");
    let memory_pattern = format!("/memory/tenants/{tenant}/%");
    let secret_pattern = format!("/tenants/{tenant}/users/%/secrets/%/secrets/%.json");
    let identity_pattern = format!("/tenants/{tenant}/shared/reborn-identity/external/%");
    let user_pattern = format!("/tenants/{tenant}/shared/reborn-identity/users/%.json");
    let project_pattern = format!("/tenants/{tenant}/shared/reborn-projects/%/records/%.json");

    match target {
        #[cfg(feature = "libsql")]
        TargetStore::LibSql { path } => {
            if !path.is_file() {
                return Err(MigrationError::OpenTarget(
                    "Reborn target does not exist for verification".to_string(),
                ));
            }
            let database = libsql::Builder::new_local(path)
                .flags(libsql::OpenFlags::SQLITE_OPEN_READ_ONLY)
                .build()
                .await
                .map_err(|error| MigrationError::OpenTarget(error.to_string()))?;
            let connection = database
                .connect()
                .map_err(|error| MigrationError::OpenTarget(error.to_string()))?;
            connection
                .execute("PRAGMA query_only = ON", ())
                .await
                .map_err(|error| MigrationError::OpenTarget(error.to_string()))?;
            Ok(TargetReadback {
                users: count_libsql(&connection, "root_filesystem_entries", &user_pattern).await?,
                threads: count_libsql(&connection, "root_filesystem_entries", &thread_pattern)
                    .await?,
                messages: count_libsql(&connection, "root_filesystem_entries", &message_pattern)
                    .await?
                    + count_libsql(&connection, "root_filesystem_events", &append_pattern).await?,
                projects: count_libsql(&connection, "root_filesystem_entries", &project_pattern)
                    .await?,
                triggers: count_libsql_tenant(&connection, "trigger_records", tenant).await?,
                memory_documents: count_libsql_files(&connection, &memory_pattern).await?,
                secrets: count_libsql(&connection, "root_filesystem_entries", &secret_pattern)
                    .await?,
                identity_records: count_libsql(
                    &connection,
                    "root_filesystem_entries",
                    &identity_pattern,
                )
                .await?,
            })
        }
        #[cfg(not(feature = "libsql"))]
        TargetStore::LibSql { .. } => Err(MigrationError::OpenTarget(
            "binary built without the libsql feature".to_string(),
        )),
        #[cfg(feature = "postgres")]
        TargetStore::Postgres { url } => {
            let pool = open_postgres_pool(url)?;
            let client = pool
                .get()
                .await
                .map_err(|error| MigrationError::OpenTarget(error.to_string()))?;
            Ok(TargetReadback {
                users: count_postgres(&client, "root_filesystem_entries", &user_pattern).await?,
                threads: count_postgres(&client, "root_filesystem_entries", &thread_pattern)
                    .await?,
                messages: count_postgres(&client, "root_filesystem_entries", &message_pattern)
                    .await?
                    + count_postgres(&client, "root_filesystem_events", &append_pattern).await?,
                projects: count_postgres(&client, "root_filesystem_entries", &project_pattern)
                    .await?,
                triggers: count_postgres_tenant(&client, "trigger_records", tenant).await?,
                memory_documents: count_postgres_files(&client, &memory_pattern).await?,
                secrets: count_postgres(&client, "root_filesystem_entries", &secret_pattern)
                    .await?,
                identity_records: count_postgres(
                    &client,
                    "root_filesystem_entries",
                    &identity_pattern,
                )
                .await?,
            })
        }
        #[cfg(not(feature = "postgres"))]
        TargetStore::Postgres { .. } => Err(MigrationError::OpenTarget(
            "binary built without the postgres feature".to_string(),
        )),
    }
}

#[cfg(feature = "libsql")]
async fn count_libsql(
    connection: &libsql::Connection,
    table: &str,
    pattern: &str,
) -> Result<u64, MigrationError> {
    let sql = format!("SELECT COUNT(*) FROM {table} WHERE path LIKE ?1");
    let mut rows = connection
        .query(&sql, [pattern])
        .await
        .map_err(|error| MigrationError::OpenTarget(error.to_string()))?;
    let count = rows
        .next()
        .await
        .map_err(|error| MigrationError::OpenTarget(error.to_string()))?
        .ok_or_else(|| MigrationError::OpenTarget("verification count returned no row".into()))?
        .get::<i64>(0)
        .map_err(|error| MigrationError::OpenTarget(error.to_string()))?;
    u64::try_from(count).map_err(|_| MigrationError::OpenTarget("negative target count".into()))
}

#[cfg(feature = "libsql")]
async fn count_libsql_tenant(
    connection: &libsql::Connection,
    table: &str,
    tenant: &str,
) -> Result<u64, MigrationError> {
    let sql = format!("SELECT COUNT(*) FROM {table} WHERE tenant_id = ?1");
    let mut rows = connection
        .query(&sql, [tenant])
        .await
        .map_err(|error| MigrationError::OpenTarget(error.to_string()))?;
    let count = rows
        .next()
        .await
        .map_err(|error| MigrationError::OpenTarget(error.to_string()))?
        .ok_or_else(|| MigrationError::OpenTarget("verification count returned no row".into()))?
        .get::<i64>(0)
        .map_err(|error| MigrationError::OpenTarget(error.to_string()))?;
    u64::try_from(count).map_err(|_| MigrationError::OpenTarget("negative target count".into()))
}

#[cfg(feature = "libsql")]
async fn count_libsql_files(
    connection: &libsql::Connection,
    pattern: &str,
) -> Result<u64, MigrationError> {
    let mut rows = connection
        .query(
            "SELECT COUNT(*) FROM root_filesystem_entries
             WHERE path LIKE ?1 AND is_dir = 0
               AND path NOT LIKE '%.meta'
               AND path NOT LIKE '%.versions/%'
               AND path NOT LIKE '%.chunks/%'",
            [pattern],
        )
        .await
        .map_err(|error| MigrationError::OpenTarget(error.to_string()))?;
    let count = rows
        .next()
        .await
        .map_err(|error| MigrationError::OpenTarget(error.to_string()))?
        .ok_or_else(|| MigrationError::OpenTarget("verification count returned no row".into()))?
        .get::<i64>(0)
        .map_err(|error| MigrationError::OpenTarget(error.to_string()))?;
    u64::try_from(count).map_err(|_| MigrationError::OpenTarget("negative target count".into()))
}

#[cfg(feature = "postgres")]
async fn count_postgres(
    client: &deadpool_postgres::Client,
    table: &str,
    pattern: &str,
) -> Result<u64, MigrationError> {
    let sql = format!("SELECT COUNT(*)::bigint FROM {table} WHERE path LIKE $1");
    let count: i64 = client
        .query_one(&sql, &[&pattern])
        .await
        .map_err(|error| MigrationError::OpenTarget(error.to_string()))?
        .try_get(0)
        .map_err(|error| MigrationError::OpenTarget(error.to_string()))?;
    u64::try_from(count).map_err(|_| MigrationError::OpenTarget("negative target count".into()))
}

#[cfg(feature = "postgres")]
async fn count_postgres_tenant(
    client: &deadpool_postgres::Client,
    table: &str,
    tenant: &str,
) -> Result<u64, MigrationError> {
    let sql = format!("SELECT COUNT(*)::bigint FROM {table} WHERE tenant_id = $1");
    let count: i64 = client
        .query_one(&sql, &[&tenant])
        .await
        .map_err(|error| MigrationError::OpenTarget(error.to_string()))?
        .try_get(0)
        .map_err(|error| MigrationError::OpenTarget(error.to_string()))?;
    u64::try_from(count).map_err(|_| MigrationError::OpenTarget("negative target count".into()))
}

#[cfg(feature = "postgres")]
async fn count_postgres_files(
    client: &deadpool_postgres::Client,
    pattern: &str,
) -> Result<u64, MigrationError> {
    let count: i64 = client
        .query_one(
            "SELECT COUNT(*)::bigint FROM root_filesystem_entries
             WHERE path LIKE $1 AND is_dir = FALSE
               AND path NOT LIKE '%.meta'
               AND path NOT LIKE '%.versions/%'
               AND path NOT LIKE '%.chunks/%'",
            &[&pattern],
        )
        .await
        .map_err(|error| MigrationError::OpenTarget(error.to_string()))?
        .try_get(0)
        .map_err(|error| MigrationError::OpenTarget(error.to_string()))?;
    u64::try_from(count).map_err(|_| MigrationError::OpenTarget("negative target count".into()))
}

/// The concrete Reborn backend the migration writes into. Both the KV substrate
/// and the triggers DB share this one handle.
pub(crate) enum Backend {
    #[cfg(feature = "libsql")]
    LibSql {
        root: Arc<ironclaw_filesystem::LibSqlRootFilesystem>,
        /// Shared handle for the triggers repo, which uses the raw DB (not the
        /// KV substrate). `LibSqlRootFilesystem` does not re-expose it.
        db: Arc<libsql::Database>,
    },
    #[cfg(feature = "postgres")]
    Postgres {
        root: Arc<ironclaw_filesystem::PostgresRootFilesystem>,
        pool: deadpool_postgres::Pool,
    },
}

/// Live Reborn write target: opened backend plus every constructed write
/// service and the scope migrated records are written under.
pub(crate) struct RebornTarget {
    /// Held for `identity_store` (the identity row-by-row follow-up); the other
    /// services already retain their own root/db Arcs.
    #[allow(dead_code)]
    pub(crate) backend: Backend,
    pub(crate) tenant_id: TenantId,
    pub(crate) agent_id: AgentId,
    pub(crate) thread_service: Arc<dyn SessionThreadService>,
    pub(crate) memory_service: Arc<dyn MemoryService>,
    pub(crate) project_repo: Arc<dyn ProjectRepository>,
    pub(crate) trigger_repo: Arc<dyn TriggerRepository>,
    /// Present when composition resolved the production target key.
    pub(crate) secret_store: Option<Arc<dyn SecretStore>>,
}

/// Narrow target used by the one-time extension ownership rewrite. It opens
/// only the canonical user directory and tenant-qualified installation store.
pub(crate) struct ExtensionOwnershipTarget {
    #[allow(dead_code)]
    backend: Backend,
    pub(crate) user_directory: Arc<dyn RebornUserDirectory>,
    pub(crate) extension_store: Arc<dyn ExtensionInstallationStore>,
}

impl ExtensionOwnershipTarget {
    pub(crate) async fn open(
        target: &TargetStore,
        tenant_id: &TenantId,
    ) -> Result<Self, MigrationError> {
        let backend = open_backend(target).await?;
        let root_dyn: Arc<dyn RootFilesystem> = match &backend {
            #[cfg(feature = "libsql")]
            Backend::LibSql { root, .. } => root.clone(),
            #[cfg(feature = "postgres")]
            Backend::Postgres { root, .. } => root.clone(),
        };
        let extension_store =
            ironclaw_reborn_composition::extension_installation_store_for_migration(
                root_dyn,
                Some(tenant_id),
            )
            .await
            .map_err(|error| {
                MigrationError::OpenTarget(format!("tenant extension installation store: {error}"))
            })?;
        let user_directory = match &backend {
            #[cfg(feature = "libsql")]
            Backend::LibSql { root, .. } => {
                build_extension_ownership_user_directory(root.clone(), tenant_id.clone())?
            }
            #[cfg(feature = "postgres")]
            Backend::Postgres { root, .. } => {
                build_extension_ownership_user_directory(root.clone(), tenant_id.clone())?
            }
        };

        Ok(Self {
            backend,
            user_directory,
            extension_store,
        })
    }
}

impl RebornTarget {
    pub(crate) async fn open(options: &MigrationOptions) -> Result<Self, MigrationError> {
        // Open the target only after apply preconditions (the lifecycle caller
        // enforces that ordering), then resolve the exact local-runtime key.
        // Local key generation writes beside the DB, so it must never happen in
        // plan mode. PostgreSQL keys arrive already resolved by composition.
        let backend = open_backend(&options.target).await?;
        let target_master_key = match (&options.secret_master_key, &options.target) {
            (Some(key), _) => Some(key.clone()),
            #[cfg(feature = "libsql")]
            (None, TargetStore::LibSql { path }) => Some(
                ironclaw_reborn_composition::resolve_local_migration_target_key(path).map_err(
                    |error| MigrationError::OpenTarget(format!("secrets master key: {error}")),
                )?,
            ),
            _ => None,
        };
        let crypto = match &target_master_key {
            Some(key) => Some(Arc::new(build_crypto(key)?)),
            None => None,
        };

        let (thread_service, memory_service, project_repo, secret_store) = match &backend {
            #[cfg(feature = "libsql")]
            Backend::LibSql { root, .. } => {
                build_kv_services(root.clone(), crypto.clone(), options.agent_id.clone())?
            }
            #[cfg(feature = "postgres")]
            Backend::Postgres { root, .. } => {
                build_kv_services(root.clone(), crypto.clone(), options.agent_id.clone())?
            }
        };
        let trigger_repo = build_trigger_repo(&backend).await?;

        Ok(Self {
            backend,
            tenant_id: options.tenant_id.clone(),
            agent_id: options.agent_id.clone(),
            thread_service,
            memory_service,
            project_repo,
            trigger_repo,
            secret_store,
        })
    }

    /// Build a per-user identity store. Identity records are scoped to a fixed
    /// (tenant, user, agent); the store type is generic over the concrete
    /// backend, so it is constructed here where `F` is known and returned as a
    /// trait object.
    pub(crate) fn identity_store(&self, user_id: UserId) -> Arc<dyn RebornIdentityResolver> {
        let tenant = self.tenant_id.clone();
        let agent = self.agent_id.clone();
        match &self.backend {
            #[cfg(feature = "libsql")]
            Backend::LibSql { root, .. } => {
                build_identity_store(root.clone(), tenant, user_id, agent)
            }
            #[cfg(feature = "postgres")]
            Backend::Postgres { root, .. } => {
                build_identity_store(root.clone(), tenant, user_id, agent)
            }
        }
    }

    /// Build the canonical user-directory port over the production identity
    /// mount. The supplied caller id only fills the store's per-user scope;
    /// canonical user records themselves live in the tenant-shared directory.
    pub(crate) fn user_directory(&self, caller_id: UserId) -> Arc<dyn RebornUserDirectory> {
        let tenant = self.tenant_id.clone();
        let agent = self.agent_id.clone();
        match &self.backend {
            #[cfg(feature = "libsql")]
            Backend::LibSql { root, .. } => {
                build_user_directory(root.clone(), tenant, caller_id, agent)
            }
            #[cfg(feature = "postgres")]
            Backend::Postgres { root, .. } => {
                build_user_directory(root.clone(), tenant, caller_id, agent)
            }
        }
    }

    /// Create a trigger only when the deterministic target slot is empty.
    /// Replays accept an exact match; a different record at the same stable id
    /// is a migration conflict and must never be overwritten by `upsert`.
    pub(crate) async fn compare_and_upsert_trigger(
        &self,
        source_id: &str,
        record: ironclaw_triggers::TriggerRecord,
    ) -> Result<(), MigrationError> {
        let existing = self
            .trigger_repo
            .get_trigger(record.tenant_id.clone(), record.trigger_id)
            .await
            .map_err(|error| MigrationError::WriteTarget {
                domain: format!("trigger for {source_id}"),
                reason: format!("read deterministic target slot: {error}"),
            })?;
        match existing {
            Some(existing) if existing == record => Ok(()),
            Some(_) => Err(MigrationError::WriteTarget {
                domain: format!("trigger for {source_id}"),
                reason: format!(
                    "deterministic trigger id {} already contains divergent state; refusing to overwrite",
                    record.trigger_id
                ),
            }),
            None => self
                .trigger_repo
                .upsert_trigger(record)
                .await
                .map_err(|error| MigrationError::WriteTarget {
                    domain: format!("trigger for {source_id}"),
                    reason: error.to_string(),
                }),
        }
    }
}

fn build_crypto(key: &SecretString) -> Result<SecretsCrypto, MigrationError> {
    SecretsCrypto::new(key.clone())
        .map_err(|e| MigrationError::OpenTarget(format!("secrets master key: {e}")))
}

/// The KV-substrate write services built over one backend.
type KvServices = (
    Arc<dyn SessionThreadService>,
    Arc<dyn MemoryService>,
    Arc<dyn ProjectRepository>,
    Option<Arc<dyn SecretStore>>,
);

/// Build the filesystem-backed KV services over one concrete backend, returning
/// them as trait objects.
fn build_kv_services<F>(
    root: Arc<F>,
    crypto: Option<Arc<SecretsCrypto>>,
    agent_id: AgentId,
) -> Result<KvServices, MigrationError>
where
    F: RootFilesystem + 'static,
{
    let scoped = Arc::new(ScopedFilesystem::new(
        root.clone(),
        mounts::production_mount_view,
    ));
    let thread_service: Arc<dyn SessionThreadService> =
        Arc::new(FilesystemSessionThreadService::new(scoped.clone()));

    let migration_user = UserId::new("reborn-migration").map_err(|error| {
        MigrationError::OpenTarget(format!("migration project repository caller: {error}"))
    })?;
    let project_repo: Arc<dyn ProjectRepository> = Arc::new(FilesystemProjectRepository::new(
        scoped.clone(),
        migration_user,
        agent_id,
    ));

    let root_dyn: Arc<dyn RootFilesystem> = root.clone();
    let memory_service: Arc<dyn MemoryService> =
        Arc::new(NativeMemoryService::from_filesystem(root_dyn, None));

    let secret_store: Option<Arc<dyn SecretStore>> = crypto.map(|crypto| {
        let store: Arc<dyn SecretStore> = Arc::new(FilesystemSecretStore::new(scoped, crypto));
        store
    });

    Ok((thread_service, memory_service, project_repo, secret_store))
}

#[allow(dead_code)] // wired for the identity row-by-row follow-up
fn build_identity_store<F>(
    root: Arc<F>,
    tenant_id: TenantId,
    user_id: UserId,
    agent_id: AgentId,
) -> Arc<dyn RebornIdentityResolver>
where
    F: RootFilesystem + 'static,
{
    let scoped = Arc::new(ScopedFilesystem::new(root, mounts::production_mount_view));
    let project_id: Option<ProjectId> = None;
    let store: Arc<dyn RebornIdentityResolver> = Arc::new(FilesystemRebornIdentityStore::new(
        scoped, tenant_id, user_id, agent_id, project_id,
    ));
    store
}

fn build_user_directory<F>(
    root: Arc<F>,
    tenant_id: TenantId,
    caller_id: UserId,
    agent_id: AgentId,
) -> Arc<dyn RebornUserDirectory>
where
    F: RootFilesystem + 'static,
{
    let scoped = Arc::new(ScopedFilesystem::new(root, mounts::production_mount_view));
    Arc::new(FilesystemRebornIdentityStore::new(
        scoped, tenant_id, caller_id, agent_id, None,
    ))
}

fn build_extension_ownership_user_directory<F>(
    root: Arc<F>,
    tenant_id: TenantId,
) -> Result<Arc<dyn RebornUserDirectory>, MigrationError>
where
    F: RootFilesystem + 'static,
{
    let actor_user_id = UserId::new("extension-ownership-migration")
        .map_err(|error| MigrationError::OpenTarget(error.to_string()))?;
    let agent_id = AgentId::new("extension-ownership-migration")
        .map_err(|error| MigrationError::OpenTarget(error.to_string()))?;
    let scoped = Arc::new(ScopedFilesystem::new(
        root,
        ironclaw_reborn_composition::invocation_mount_view,
    ));
    let store: Arc<dyn RebornUserDirectory> = Arc::new(FilesystemRebornIdentityStore::new(
        scoped,
        tenant_id,
        actor_user_id,
        agent_id,
        None,
    ));
    Ok(store)
}

async fn build_trigger_repo(
    backend: &Backend,
) -> Result<Arc<dyn TriggerRepository>, MigrationError> {
    match backend {
        #[cfg(feature = "libsql")]
        Backend::LibSql { db, .. } => {
            let repo = ironclaw_triggers::LibSqlTriggerRepository::new(db.clone());
            repo.run_migrations()
                .await
                .map_err(|e| MigrationError::OpenTarget(format!("trigger migrations: {e}")))?;
            let repo: Arc<dyn TriggerRepository> = Arc::new(repo);
            Ok(repo)
        }
        #[cfg(feature = "postgres")]
        Backend::Postgres { pool, .. } => {
            let repo = ironclaw_triggers::PostgresTriggerRepository::new(pool.clone());
            repo.run_migrations()
                .await
                .map_err(|e| MigrationError::OpenTarget(format!("trigger migrations: {e}")))?;
            let repo: Arc<dyn TriggerRepository> = Arc::new(repo);
            Ok(repo)
        }
    }
}

async fn open_backend(target: &TargetStore) -> Result<Backend, MigrationError> {
    match target {
        #[cfg(feature = "libsql")]
        TargetStore::LibSql { path } => {
            if let Some(parent) = path.parent() {
                tokio::fs::create_dir_all(parent).await?;
            }
            let db = Arc::new(
                libsql::Builder::new_local(path)
                    .build()
                    .await
                    .map_err(|e| MigrationError::OpenTarget(e.to_string()))?,
            );
            let root = Arc::new(ironclaw_filesystem::LibSqlRootFilesystem::new(db.clone()));
            root.run_migrations()
                .await
                .map_err(|e| MigrationError::OpenTarget(e.to_string()))?;
            Ok(Backend::LibSql { root, db })
        }
        #[cfg(not(feature = "libsql"))]
        TargetStore::LibSql { .. } => Err(MigrationError::OpenTarget(
            "binary built without the libsql feature".into(),
        )),
        #[cfg(feature = "postgres")]
        TargetStore::Postgres { url } => {
            let pool = open_postgres_pool(url)?;
            let root = Arc::new(ironclaw_filesystem::PostgresRootFilesystem::new(
                pool.clone(),
            ));
            root.run_migrations()
                .await
                .map_err(|e| MigrationError::OpenTarget(e.to_string()))?;
            Ok(Backend::Postgres { root, pool })
        }
        #[cfg(not(feature = "postgres"))]
        TargetStore::Postgres { .. } => Err(MigrationError::OpenTarget(
            "binary built without the postgres feature".into(),
        )),
    }
}

/// Build the Reborn target Postgres pool with the repo's remote-TLS rule:
/// remote hosts must use TLS (mirrors `ironclaw_reborn_event_store` and
/// `src/db/tls.rs`). A remote `sslmode=disable` is rejected rather than sending
/// migration traffic — including decrypted secrets — in cleartext; local
/// connections keep plain TCP. TLS wiring is reused from `ironclaw::db::tls`.
#[cfg(feature = "postgres")]
fn open_postgres_pool(
    url: &secrecy::SecretString,
) -> Result<deadpool_postgres::Pool, MigrationError> {
    use secrecy::ExposeSecret;

    let raw = url.expose_secret();
    let pg_config = raw
        .parse::<tokio_postgres::Config>()
        .map_err(|e| MigrationError::OpenTarget(format!("parse Postgres URL: {e}")))?;
    let remote = !is_local_postgres_config(&pg_config);
    let ssl_mode = match pg_config.get_ssl_mode() {
        tokio_postgres::config::SslMode::Disable => {
            if remote {
                return Err(MigrationError::OpenTarget(
                    "remote Postgres target requires TLS; sslmode=disable is rejected for \
                     migration traffic (it carries decrypted secrets)"
                        .into(),
                ));
            }
            ironclaw::config::SslMode::Disable
        }
        // `Prefer`/`Require`/future variants: force TLS on remote, allow the
        // parsed intent on local.
        _ if remote => ironclaw::config::SslMode::Require,
        _ => ironclaw::config::SslMode::Prefer,
    };

    let mut dp_config = deadpool_postgres::Config::new();
    dp_config.url = Some(raw.to_string());
    ironclaw::db::tls::create_pool(&dp_config, ssl_mode)
        .map_err(|e| MigrationError::OpenTarget(e.to_string()))
}

#[cfg(feature = "postgres")]
pub(crate) async fn postgres_stores_are_distinct(
    source_pool: &deadpool_postgres::Pool,
    target_url: &secrecy::SecretString,
) -> Result<bool, MigrationError> {
    const LOCK_NAMESPACE: i64 = 0x4943_4d47;

    let source = source_pool.get().await.map_err(postgres_identity_error)?;
    let target_pool = open_postgres_pool(target_url)?;
    let target = target_pool.get().await.map_err(postgres_identity_error)?;
    let source_database_oid: i64 = source
        .query_one(
            "SELECT oid::bigint FROM pg_database WHERE datname = current_database()",
            &[],
        )
        .await
        .map_err(postgres_identity_error)?
        .get(0);
    let target_database_oid: i64 = target
        .query_one(
            "SELECT oid::bigint FROM pg_database WHERE datname = current_database()",
            &[],
        )
        .await
        .map_err(postgres_identity_error)?
        .get(0);
    let source_lock_key = (LOCK_NAMESPACE << 32) | (source_database_oid & 0xffff_ffff);
    let target_lock_key = (LOCK_NAMESPACE << 32) | (target_database_oid & 0xffff_ffff);
    let source_locked: bool = source
        .query_one("SELECT pg_try_advisory_lock($1)", &[&source_lock_key])
        .await
        .map_err(postgres_identity_error)?
        .get(0);
    if !source_locked {
        return Err(MigrationError::OpenSource(
            "could not establish PostgreSQL source identity lock".to_string(),
        ));
    }

    let target_locked = target
        .query_one("SELECT pg_try_advisory_lock($1)", &[&target_lock_key])
        .await
        .map(|row| row.get::<_, bool>(0));
    let distinct = match target_locked {
        Ok(locked) => {
            if locked {
                let _ = target
                    .query_one("SELECT pg_advisory_unlock($1)", &[&target_lock_key])
                    .await;
            }
            locked
        }
        Err(error) => {
            let _ = source
                .query_one("SELECT pg_advisory_unlock($1)", &[&source_lock_key])
                .await;
            return Err(postgres_identity_error(error));
        }
    };
    source
        .query_one("SELECT pg_advisory_unlock($1)", &[&source_lock_key])
        .await
        .map_err(postgres_identity_error)?;
    Ok(distinct)
}

#[cfg(feature = "postgres")]
fn postgres_identity_error(error: impl std::fmt::Display) -> MigrationError {
    MigrationError::OpenTarget(format!(
        "PostgreSQL store identity probe failed (connection details redacted): {error}"
    ))
}

/// True when the parsed Postgres `Config` targets only loopback hosts / Unix
/// sockets. Anything else is treated as remote and must use TLS. Mirrors the
/// event-store's `is_local_postgres_config`.
#[cfg(feature = "postgres")]
fn is_local_postgres_config(config: &tokio_postgres::Config) -> bool {
    use tokio_postgres::config::Host;

    let hosts = config.get_hosts();
    let hostaddrs = config.get_hostaddrs();
    if hosts.is_empty() && hostaddrs.is_empty() {
        // Empty host list means libpq's compiled-in default socket directory.
        return true;
    }
    for host in hosts {
        match host {
            #[cfg(unix)]
            Host::Unix(_) => continue,
            Host::Tcp(name) => {
                if !matches!(
                    name.as_str(),
                    "localhost" | "127.0.0.1" | "::1" | "[::1]" | "0.0.0.0"
                ) {
                    return false;
                }
            }
        }
    }
    for addr in hostaddrs {
        if !addr.is_loopback() && !addr.is_unspecified() {
            return false;
        }
    }
    true
}

#[cfg(all(test, feature = "postgres"))]
mod tests {
    use super::shared_state_transition_allowed;
    use crate::manifest::MigrationStatus;

    #[test]
    fn shared_state_accepts_replay_and_manifest_lifecycle_transitions() {
        assert!(shared_state_transition_allowed(
            "applying",
            MigrationStatus::Applying
        ));
        assert!(shared_state_transition_allowed(
            "applying",
            MigrationStatus::Applied
        ));
        assert!(shared_state_transition_allowed(
            "failed",
            MigrationStatus::Applying
        ));
        assert!(shared_state_transition_allowed(
            "applied",
            MigrationStatus::Verifying
        ));
        assert!(shared_state_transition_allowed(
            "verifying",
            MigrationStatus::Verified
        ));
        assert!(shared_state_transition_allowed(
            "verified",
            MigrationStatus::Verified
        ));
        assert!(shared_state_transition_allowed(
            "verified",
            MigrationStatus::Verifying
        ));
    }

    #[test]
    fn shared_state_rejects_skipped_and_unknown_transitions() {
        assert!(!shared_state_transition_allowed(
            "applying",
            MigrationStatus::Verified
        ));
        assert!(!shared_state_transition_allowed(
            "failed",
            MigrationStatus::Verified
        ));
        assert!(!shared_state_transition_allowed(
            "verified",
            MigrationStatus::Applying
        ));
        assert!(!shared_state_transition_allowed(
            "unknown",
            MigrationStatus::Applying
        ));
    }
}
