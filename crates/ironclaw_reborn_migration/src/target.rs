//! Reborn target access for operator migrations.

use std::sync::Arc;

use ironclaw_extensions::ExtensionInstallationStore;
use ironclaw_filesystem::{RootFilesystem, ScopedFilesystem};
use ironclaw_host_api::{AgentId, TenantId, UserId};
use ironclaw_reborn_identity::{FilesystemRebornIdentityStore, RebornUserDirectory};

use crate::error::MigrationError;
use crate::options::TargetStore;

/// The concrete Reborn backend the migration writes into.
pub(crate) enum Backend {
    #[cfg(feature = "libsql")]
    LibSql {
        root: Arc<ironclaw_filesystem::LibSqlRootFilesystem>,
    },
    #[cfg(feature = "postgres")]
    Postgres {
        root: Arc<ironclaw_filesystem::PostgresRootFilesystem>,
    },
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
            Backend::LibSql { root } => root.clone(),
            #[cfg(feature = "postgres")]
            Backend::Postgres { root } => root.clone(),
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
            Backend::LibSql { root } => build_user_directory(root.clone(), tenant_id.clone())?,
            #[cfg(feature = "postgres")]
            Backend::Postgres { root } => build_user_directory(root.clone(), tenant_id.clone())?,
        };

        Ok(Self {
            backend,
            user_directory,
            extension_store,
        })
    }
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
                    .map_err(|error| MigrationError::OpenTarget(error.to_string()))?,
            );
            let root = Arc::new(ironclaw_filesystem::LibSqlRootFilesystem::new(db));
            root.run_migrations()
                .await
                .map_err(|error| MigrationError::OpenTarget(error.to_string()))?;
            Ok(Backend::LibSql { root })
        }
        #[cfg(not(feature = "libsql"))]
        TargetStore::LibSql { .. } => Err(MigrationError::OpenTarget(
            "binary built without the libsql feature".into(),
        )),
        #[cfg(feature = "postgres")]
        TargetStore::Postgres { url } => {
            let pool = open_postgres_pool(url)?;
            let root = Arc::new(ironclaw_filesystem::PostgresRootFilesystem::new(pool));
            root.run_migrations()
                .await
                .map_err(|error| MigrationError::OpenTarget(error.to_string()))?;
            Ok(Backend::Postgres { root })
        }
        #[cfg(not(feature = "postgres"))]
        TargetStore::Postgres { .. } => Err(MigrationError::OpenTarget(
            "binary built without the postgres feature".into(),
        )),
    }
}

/// Build the Reborn target Postgres pool through the production composition
/// helper so migrations inherit the same fail-closed remote-TLS policy.
#[cfg(feature = "postgres")]
fn open_postgres_pool(
    url: &secrecy::SecretString,
) -> Result<deadpool_postgres::Pool, MigrationError> {
    ironclaw_reborn_composition::open_reborn_postgres_pool(url.clone())
        .map_err(|error| MigrationError::OpenTarget(error.to_string()))
}
