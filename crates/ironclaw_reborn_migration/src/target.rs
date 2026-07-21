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

use ironclaw_host_api::ProjectId;
use ironclaw_memory_native::{
    ChunkingMemoryDocumentIndexer, DefaultPromptWriteSafetyPolicy,
    FilesystemMemoryDocumentRepository, MemoryBackend, MemoryBackendCapabilities,
    PromptProtectedPathRegistry, PromptSafetyPolicyVersion, RepositoryMemoryBackend,
};
use ironclaw_reborn_identity::RebornIdentityResolver;
use ironclaw_secrets::{FilesystemSecretStore, SecretStore, SecretsCrypto};
use ironclaw_threads::{FilesystemSessionThreadService, SessionThreadService};
use ironclaw_triggers::TriggerRepository;
use secrecy::SecretString;

use crate::error::MigrationError;
use crate::options::TargetStore;

use crate::mounts;
use crate::options::MigrationOptions;

/// The concrete Reborn backend the migration writes into. Both the KV substrate
/// and the triggers DB share this one handle.
pub(crate) enum Backend {
    LibSql {
        root: Arc<ironclaw_filesystem::LibSqlRootFilesystem>,
        /// Shared handle for the triggers repo, which uses the raw DB (not the
        /// KV substrate). `LibSqlRootFilesystem` does not re-expose it.
        db: Arc<libsql::Database>,
    },
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
    pub(crate) memory_backend: Arc<dyn MemoryBackend>,
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
            Backend::LibSql { root, .. } => root.clone(),
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
            Backend::LibSql { root, .. } => build_user_directory(root.clone(), tenant_id.clone())?,
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

impl RebornTarget {
    pub(crate) async fn open(options: &MigrationOptions) -> Result<Self, MigrationError> {
        let crypto = match &options.secret_master_key {
            Some(key) => Some(Arc::new(build_crypto(key)?)),
            None => None,
        };

        let backend = open_backend(&options.target).await?;
        let (thread_service, memory_backend, secret_store) = match &backend {
            Backend::LibSql { root, .. } => build_kv_services(root.clone(), crypto.clone())?,
            Backend::Postgres { root, .. } => build_kv_services(root.clone(), crypto.clone())?,
        };
        let trigger_repo = build_trigger_repo(&backend).await?;

        // Extension installation store is owned by composition; the migration
        // seam builds it over our root filesystem at the default state path.
        let root_dyn: Arc<dyn RootFilesystem> = match &backend {
            Backend::LibSql { root, .. } => root.clone(),
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
            memory_backend,
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
            Backend::LibSql { root, .. } => {
                build_identity_store(root.clone(), tenant, user_id, agent)
            }
            Backend::Postgres { root, .. } => {
                build_identity_store(root.clone(), tenant, user_id, agent)
            }
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
    Arc<dyn MemoryBackend>,
    Option<Arc<dyn SecretStore>>,
);

/// Build the filesystem-backed KV services over one concrete backend, returning
/// them as trait objects.
fn build_kv_services<F>(
    root: Arc<F>,
    crypto: Option<Arc<SecretsCrypto>>,
) -> Result<KvServices, MigrationError>
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
    let memory_backend = build_memory_backend(root_dyn)?;

    let secret_store: Option<Arc<dyn SecretStore>> = crypto.map(|crypto| {
        let secrets_scoped = Arc::new(ScopedFilesystem::new(
            root.clone(),
            mounts::secrets_mount_view,
        ));
        let store: Arc<dyn SecretStore> =
            Arc::new(FilesystemSecretStore::new(secrets_scoped, crypto));
        store
    });

    Ok((thread_service, memory_backend, secret_store))
}

fn build_memory_backend(
    filesystem: Arc<dyn RootFilesystem>,
) -> Result<Arc<dyn MemoryBackend>, MigrationError> {
    let repository = Arc::new(FilesystemMemoryDocumentRepository::new(filesystem));
    let indexer = Arc::new(ChunkingMemoryDocumentIndexer::new(repository.clone()));
    // Intentionally empty, and load-bearing: the migration must be able to
    // import legacy content into paths that are normally prompt-protected (e.g.
    // BOOTSTRAP.md). Because `MemoryBackendWriteOptions` leaves
    // `prompt_safety_already_enforced: false`, the backend consults this
    // registry on every write and is fail-closed by default; an empty registry
    // is what makes it match nothing, so no legacy write is blocked. This
    // bypass is deliberately scoped to this ephemeral, one-shot migration-only
    // backend — do NOT "fix" it by populating protected paths here, or you will
    // silently reintroduce write failures for protected legacy documents.
    let empty_protected_paths = PromptProtectedPathRegistry::new(
        PromptSafetyPolicyVersion::new("migration-empty-prompt-protected-paths:v1")
            .map_err(|error| MigrationError::OpenTarget(error.to_string()))?,
        std::iter::empty::<String>(),
    )
    .map_err(|error| MigrationError::OpenTarget(error.to_string()))?;
    Ok(Arc::new(
        RepositoryMemoryBackend::new(repository)
            .with_indexer(indexer)
            .with_prompt_write_safety_policy(Arc::new(
                DefaultPromptWriteSafetyPolicy::with_registry(empty_protected_paths.clone()),
            ))
            .with_prompt_protected_path_registry(empty_protected_paths)
            .with_capabilities(
                MemoryBackendCapabilities::default()
                    .set_file_documents(true)
                    .set_metadata(true)
                    .set_versioning(true)
                    .set_prompt_write_safety(true)
                    .set_full_text_search(true)
                    .set_delete(true)
                    .set_transactions(true),
            ),
    ))
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

async fn build_trigger_repo(
    backend: &Backend,
) -> Result<Arc<dyn TriggerRepository>, MigrationError> {
    match backend {
        Backend::LibSql { db, .. } => {
            let repo = ironclaw_triggers::LibSqlTriggerRepository::new(db.clone());
            repo.run_migrations()
                .await
                .map_err(|e| MigrationError::OpenTarget(format!("trigger migrations: {e}")))?;
            let repo: Arc<dyn TriggerRepository> = Arc::new(repo);
            Ok(repo)
        }
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
    }
}

/// Build the Reborn target Postgres pool through the production composition
/// helper so migrations inherit the same fail-closed remote-TLS policy without
/// linking the legacy root crate.
fn open_postgres_pool(
    url: &secrecy::SecretString,
) -> Result<deadpool_postgres::Pool, MigrationError> {
    ironclaw_reborn_composition::open_reborn_postgres_pool(url.clone())
        .map_err(|error| MigrationError::OpenTarget(error.to_string()))
}
