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
use ironclaw_host_api::{AgentId, TenantId, UserId};
use ironclaw_reborn_identity::{FilesystemRebornIdentityStore, RebornUserDirectory};

#[cfg(feature = "full-migration")]
use ironclaw_host_api::ProjectId;
#[cfg(feature = "full-migration")]
use ironclaw_memory::MemoryService;
#[cfg(feature = "full-migration")]
use ironclaw_memory_native::NativeMemoryService;
#[cfg(feature = "full-migration")]
use ironclaw_reborn_identity::RebornIdentityResolver;
#[cfg(feature = "full-migration")]
use ironclaw_secrets::{FilesystemSecretStore, SecretStore, SecretsCrypto};
#[cfg(feature = "full-migration")]
use ironclaw_threads::{FilesystemSessionThreadService, SessionThreadService};
#[cfg(feature = "full-migration")]
use ironclaw_triggers::TriggerRepository;
#[cfg(feature = "full-migration")]
use secrecy::SecretString;

use crate::error::MigrationError;
use crate::options::TargetStore;

#[cfg(feature = "full-migration")]
use crate::mounts;
#[cfg(feature = "full-migration")]
use crate::options::MigrationOptions;

/// The concrete Reborn backend the migration writes into. Both the KV substrate
/// and the triggers DB share this one handle.
pub(crate) enum Backend {
    #[cfg(feature = "libsql")]
    LibSql {
        root: Arc<ironclaw_filesystem::LibSqlRootFilesystem>,
        /// Shared handle for the triggers repo, which uses the raw DB (not the
        /// KV substrate). `LibSqlRootFilesystem` does not re-expose it.
        #[cfg(feature = "full-migration")]
        db: Arc<libsql::Database>,
    },
    #[cfg(feature = "postgres")]
    Postgres {
        root: Arc<ironclaw_filesystem::PostgresRootFilesystem>,
        #[cfg(feature = "full-migration")]
        pool: deadpool_postgres::Pool,
    },
}

/// Live Reborn write target: opened backend plus every constructed write
/// service and the scope migrated records are written under.
#[cfg(feature = "full-migration")]
pub(crate) struct RebornTarget {
    /// Held for `identity_store` (the identity row-by-row follow-up); the other
    /// services already retain their own root/db Arcs.
    #[allow(dead_code)]
    pub(crate) backend: Backend,
    pub(crate) tenant_id: TenantId,
    pub(crate) agent_id: AgentId,
    pub(crate) thread_service: Arc<dyn SessionThreadService>,
    pub(crate) memory_service: Arc<dyn MemoryService>,
    pub(crate) trigger_repo: Arc<dyn TriggerRepository>,
    pub(crate) extension_store: Arc<dyn ExtensionInstallationStore>,
    /// Present only when a secrets master key was supplied.
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
            Backend::LibSql { root, .. } => build_user_directory(root.clone(), tenant_id.clone())?,
            #[cfg(feature = "postgres")]
            Backend::Postgres { root, .. } => {
                build_user_directory(root.clone(), tenant_id.clone())?
            }
        };

        Ok(Self {
            backend,
            user_directory,
            extension_store,
        })
    }
}

#[cfg(feature = "full-migration")]
impl RebornTarget {
    pub(crate) async fn open(options: &MigrationOptions) -> Result<Self, MigrationError> {
        let crypto = match &options.secret_master_key {
            Some(key) => Some(Arc::new(build_crypto(key)?)),
            None => None,
        };

        let backend = open_backend(&options.target).await?;
        let (thread_service, memory_service, secret_store) = match &backend {
            #[cfg(feature = "libsql")]
            Backend::LibSql { root, .. } => build_kv_services(root.clone(), crypto.clone()),
            #[cfg(feature = "postgres")]
            Backend::Postgres { root, .. } => build_kv_services(root.clone(), crypto.clone()),
        };
        let trigger_repo = build_trigger_repo(&backend).await?;

        // Extension installation store is owned by composition; the migration
        // seam builds it over our root filesystem at the default state path.
        let root_dyn: Arc<dyn RootFilesystem> = match &backend {
            #[cfg(feature = "libsql")]
            Backend::LibSql { root, .. } => root.clone(),
            #[cfg(feature = "postgres")]
            Backend::Postgres { root, .. } => root.clone(),
        };
        let extension_store =
            ironclaw_reborn_composition::extension_installation_store_for_migration(root_dyn, None)
                .await
                .map_err(|e| {
                    MigrationError::OpenTarget(format!("extension installation store: {e}"))
                })?;

        Ok(Self {
            backend,
            tenant_id: options.tenant_id.clone(),
            agent_id: options.agent_id.clone(),
            thread_service,
            memory_service,
            trigger_repo,
            extension_store,
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
}

#[cfg(feature = "full-migration")]
fn build_crypto(key: &SecretString) -> Result<SecretsCrypto, MigrationError> {
    SecretsCrypto::new(key.clone())
        .map_err(|e| MigrationError::OpenTarget(format!("secrets master key: {e}")))
}

/// The KV-substrate write services built over one backend.
#[cfg(feature = "full-migration")]
type KvServices = (
    Arc<dyn SessionThreadService>,
    Arc<dyn MemoryService>,
    Option<Arc<dyn SecretStore>>,
);

/// Build the filesystem-backed KV services over one concrete backend, returning
/// them as trait objects.
#[cfg(feature = "full-migration")]
fn build_kv_services<F>(root: Arc<F>, crypto: Option<Arc<SecretsCrypto>>) -> KvServices
where
    F: RootFilesystem + 'static,
{
    let threads_scoped = Arc::new(ScopedFilesystem::new(
        root.clone(),
        mounts::threads_mount_view,
    ));
    let thread_service: Arc<dyn SessionThreadService> =
        Arc::new(FilesystemSessionThreadService::new(threads_scoped));

    let root_dyn: Arc<dyn RootFilesystem> = root.clone();
    let memory_service: Arc<dyn MemoryService> =
        Arc::new(NativeMemoryService::from_filesystem(root_dyn, None));

    let secret_store: Option<Arc<dyn SecretStore>> = crypto.map(|crypto| {
        let secrets_scoped = Arc::new(ScopedFilesystem::new(
            root.clone(),
            mounts::secrets_mount_view,
        ));
        let store: Arc<dyn SecretStore> =
            Arc::new(FilesystemSecretStore::new(secrets_scoped, crypto));
        store
    });

    (thread_service, memory_service, secret_store)
}

#[cfg(feature = "full-migration")]
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
    let scoped = Arc::new(ScopedFilesystem::new(root, mounts::identity_mount_view));
    let project_id: Option<ProjectId> = None;
    let store: Arc<dyn RebornIdentityResolver> = Arc::new(FilesystemRebornIdentityStore::new(
        scoped, tenant_id, user_id, agent_id, project_id,
    ));
    store
}

fn build_user_directory<F>(
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

#[cfg(feature = "full-migration")]
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
            Ok(Backend::LibSql {
                root,
                #[cfg(feature = "full-migration")]
                db,
            })
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
            Ok(Backend::Postgres {
                root,
                #[cfg(feature = "full-migration")]
                pool,
            })
        }
        #[cfg(not(feature = "postgres"))]
        TargetStore::Postgres { .. } => Err(MigrationError::OpenTarget(
            "binary built without the postgres feature".into(),
        )),
    }
}

/// Build the Reborn target Postgres pool through the production composition
/// helper so migrations inherit the same fail-closed remote-TLS policy without
/// linking the legacy root crate.
#[cfg(feature = "postgres")]
fn open_postgres_pool(
    url: &secrecy::SecretString,
) -> Result<deadpool_postgres::Pool, MigrationError> {
    ironclaw_reborn_composition::open_reborn_postgres_pool(url.clone())
        .map_err(|error| MigrationError::OpenTarget(error.to_string()))
}
