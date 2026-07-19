//! Reborn target access for operator migrations.

use std::{fmt::Display, sync::Arc};

use ironclaw_extensions::ExtensionInstallationStore;
use ironclaw_filesystem::{RootFilesystem, ScopedFilesystem};
use ironclaw_host_api::{AgentId, TenantId, UserId};
use ironclaw_reborn_identity::{FilesystemRebornIdentityStore, RebornUserDirectory};

use crate::error::MigrationError;
use crate::options::TargetStore;

fn open_target_error(operation: &'static str, error: impl Display) -> MigrationError {
    MigrationError::OpenTarget(format!("{operation}: {error}"))
}

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
            .map_err(|error| open_target_error("tenant extension installation store", error))?;
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
        .map_err(|error| open_target_error("migration actor user ID", error))?;
    let agent_id = AgentId::new("extension-ownership-migration")
        .map_err(|error| open_target_error("migration agent ID", error))?;
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
                tokio::fs::create_dir_all(parent)
                    .await
                    .map_err(|error| open_target_error("create libSQL parent directory", error))?;
            }
            let db = Arc::new(
                libsql::Builder::new_local(path)
                    .build()
                    .await
                    .map_err(|error| open_target_error("open libSQL database", error))?,
            );
            let root = Arc::new(ironclaw_filesystem::LibSqlRootFilesystem::new(db));
            root.run_migrations()
                .await
                .map_err(|error| open_target_error("run libSQL filesystem migrations", error))?;
            Ok(Backend::LibSql { root })
        }
        #[cfg(not(feature = "libsql"))]
        TargetStore::LibSql { .. } => Err(open_target_error(
            "select libSQL backend",
            "binary built without the libsql feature",
        )),
        #[cfg(feature = "postgres")]
        TargetStore::Postgres { url } => {
            let pool = open_postgres_pool(url)?;
            let root = Arc::new(ironclaw_filesystem::PostgresRootFilesystem::new(pool));
            root.run_migrations().await.map_err(|error| {
                open_target_error("run PostgreSQL filesystem migrations", error)
            })?;
            Ok(Backend::Postgres { root })
        }
        #[cfg(not(feature = "postgres"))]
        TargetStore::Postgres { .. } => Err(open_target_error(
            "select PostgreSQL backend",
            "binary built without the postgres feature",
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
        .map_err(|error| open_target_error("open PostgreSQL connection pool", error))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_open_error_preserves_operation_and_reason() {
        let error = open_target_error("run libSQL filesystem migrations", "schema rejected");

        assert_eq!(
            error.to_string(),
            "failed to open Reborn target store: run libSQL filesystem migrations: schema rejected"
        );
    }

    #[cfg(feature = "postgres")]
    #[test]
    fn postgres_pool_error_identifies_the_failing_operation() {
        let url = secrecy::SecretString::from("not a PostgreSQL URL");
        let error = match open_postgres_pool(&url) {
            Ok(_) => panic!("invalid PostgreSQL URL unexpectedly opened a pool"),
            Err(error) => error,
        };

        assert!(
            error
                .to_string()
                .contains("open PostgreSQL connection pool:"),
            "unexpected error: {error}"
        );
    }
}
