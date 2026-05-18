use std::{fmt, sync::Arc};

use ironclaw_filesystem::{LibSqlRootFilesystem, RootFilesystem, ScopedFilesystem};
use ironclaw_host_api::{MountAlias, MountGrant, MountPermissions, MountView, VirtualPath};
use ironclaw_secrets::{
    FilesystemSecretStore, SecretError, SecretMaterial, SecretStore, SecretsCrypto,
};

/// Explicit standalone-Reborn secret store configuration.
///
/// Reborn does not auto-generate a durable master key. Production composition
/// must pass operator-controlled key material so encrypted rows remain
/// decryptable across restarts, matching v1's encrypted-at-rest invariant.
pub struct RebornLibSqlSecretStoreConfig {
    pub database: Arc<libsql::Database>,
    /// Operator-provided durable master key material.
    ///
    /// `None` is accepted only so builders and health checks can fail closed
    /// with [`RebornSecretStoreError::MissingMasterKey`]. Production
    /// composition must pass `Some`.
    pub master_key: Option<SecretMaterial>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebornSecretStoreHealth {
    pub status: RebornSecretStoreHealthStatus,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RebornSecretStoreHealthStatus {
    Ready,
    MissingMasterKey,
    InvalidMasterKey,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RebornSecretStoreError {
    MissingMasterKey,
    InvalidMasterKey,
    BackendUnavailable,
}

impl fmt::Display for RebornSecretStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingMasterKey => formatter
                .write_str("reborn secret store requires an explicit operator-provided master key"),
            Self::InvalidMasterKey => {
                formatter.write_str("reborn secret store master key is invalid")
            }
            Self::BackendUnavailable => {
                formatter.write_str("reborn secret store backend unavailable")
            }
        }
    }
}

impl std::error::Error for RebornSecretStoreError {}

/// Probe the libSQL Reborn secret-store wiring.
///
/// This uses the same fail-closed construction path as the builder, including
/// migration and secret-store key-check verification. `config.master_key = None` is
/// reported as [`RebornSecretStoreHealthStatus::MissingMasterKey`], not treated
/// as a healthy local/default configuration.
pub async fn check_libsql_reborn_secret_store_health(
    config: RebornLibSqlSecretStoreConfig,
) -> RebornSecretStoreHealth {
    match build_libsql_reborn_secret_store(config).await {
        Ok(_) => RebornSecretStoreHealth {
            status: RebornSecretStoreHealthStatus::Ready,
            reason: None,
        },
        Err(RebornSecretStoreError::MissingMasterKey) => RebornSecretStoreHealth {
            status: RebornSecretStoreHealthStatus::MissingMasterKey,
            reason: Some("explicit operator master key is required".to_string()),
        },
        Err(RebornSecretStoreError::InvalidMasterKey) => RebornSecretStoreHealth {
            status: RebornSecretStoreHealthStatus::InvalidMasterKey,
            reason: Some(
                "operator master key is invalid or cannot decrypt existing secret rows".to_string(),
            ),
        },
        Err(_) => RebornSecretStoreHealth {
            status: RebornSecretStoreHealthStatus::Unavailable,
            reason: Some("secret store backend unavailable".to_string()),
        },
    }
}

/// Build the libSQL Reborn secret store.
///
/// Requires explicit operator-provided master key material. The returned store
/// is a [`FilesystemSecretStore`] over the libSQL-backed [`RootFilesystem`];
/// the underlying `RootFilesystem` schema migration has run before this
/// returns. The FS-stored master-key sentinel was removed alongside the
/// tenant-aware `ScopedFilesystem` rework — master-key correctness is now
/// verified on first per-tenant decrypt op (see PR #3679).
pub async fn build_libsql_reborn_secret_store(
    config: RebornLibSqlSecretStoreConfig,
) -> Result<Arc<dyn SecretStore>, RebornSecretStoreError> {
    let master_key = config
        .master_key
        .ok_or(RebornSecretStoreError::MissingMasterKey)?;
    let crypto = Arc::new(SecretsCrypto::new(master_key).map_err(|error| match error {
        SecretError::InvalidMasterKey => RebornSecretStoreError::InvalidMasterKey,
        _ => RebornSecretStoreError::BackendUnavailable,
    })?);
    let filesystem = Arc::new(LibSqlRootFilesystem::new(config.database));
    filesystem
        .run_migrations()
        .await
        .map_err(|_| RebornSecretStoreError::BackendUnavailable)?;
    let scoped = reborn_singleton_secret_mount(filesystem)
        .map_err(|_| RebornSecretStoreError::BackendUnavailable)?;
    let store = FilesystemSecretStore::new(scoped, crypto);
    Ok(Arc::new(store))
}

/// Build the single-tenant `/secrets` mount the standalone Reborn binary uses
/// when wiring a secret store directly. Mirrors
/// `ironclaw_reborn_composition::default_singleton_mount_view` but is kept
/// local so this crate does not depend on the composition crate.
fn reborn_singleton_secret_mount<F>(
    filesystem: Arc<F>,
) -> Result<Arc<ScopedFilesystem<F>>, ironclaw_host_api::HostApiError>
where
    F: RootFilesystem,
{
    let view = MountView::new(vec![MountGrant::new(
        MountAlias::new("/secrets")?,
        VirtualPath::new("/secrets")?,
        MountPermissions::read_write_list_delete(),
    )])?;
    Ok(Arc::new(ScopedFilesystem::with_fixed_view(
        filesystem, view,
    )))
}
