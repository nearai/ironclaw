use std::sync::Arc;

use ironclaw_approvals::PersistentApprovalPolicyStore;
use ironclaw_authorization::CapabilityLeaseStore;
use ironclaw_filesystem::{CompositeRootFilesystem, RootFilesystem, ScopedFilesystem};
use ironclaw_resources::FilesystemResourceGovernor;
use ironclaw_secrets::{CredentialBroker, SecretStore};
use ironclaw_triggers::TriggerRepository;

use crate::factory::{
    ComposedCapabilityLeaseStore, ComposedPersistentApprovalPolicyStore, ComposedResourceGovernor,
};
use crate::filesystem_assembly::DurableBackend;
use crate::storage_catalog::validate_reborn_runtime_storage;
use crate::{RebornBuildError, RebornCompositionError};

/// Secret store and credential broker sharing one filesystem and crypto
/// authority.
pub(crate) struct SecretCredentialStores<F>
where
    F: RootFilesystem + 'static,
{
    pub(crate) secret_store: Arc<SecretStore<F>>,
    pub(crate) credential_broker: Arc<CredentialBroker<F>>,
    pub(crate) crypto: Arc<ironclaw_secrets::SecretsCrypto>,
}

impl<F> SecretCredentialStores<F>
where
    F: RootFilesystem + 'static,
{
    pub(crate) fn new(
        scoped_filesystem: Arc<ScopedFilesystem<F>>,
        crypto: Arc<ironclaw_secrets::SecretsCrypto>,
    ) -> Self {
        Self {
            secret_store: Arc::new(SecretStore::new(
                Arc::clone(&scoped_filesystem),
                Arc::clone(&crypto),
            )),
            credential_broker: Arc::new(CredentialBroker::new(
                scoped_filesystem,
                Arc::clone(&crypto),
            )),
            crypto,
        }
    }

    pub(crate) fn from_master_key(
        scoped_filesystem: Arc<ScopedFilesystem<F>>,
        master_key: ironclaw_secrets::SecretMaterial,
    ) -> Result<Self, RebornCompositionError> {
        Ok(Self::new(
            scoped_filesystem,
            Arc::new(ironclaw_secrets::SecretsCrypto::new(master_key)?),
        ))
    }
}

pub(crate) async fn build_filesystem_secret_credential_stores<F>(
    scoped_filesystem: Arc<ScopedFilesystem<F>>,
    master_key: Option<ironclaw_secrets::SecretMaterial>,
) -> Result<SecretCredentialStores<F>, RebornCompositionError>
where
    F: RootFilesystem + 'static,
{
    let master_key = resolve_explicit_or_keychain_master_key(master_key)
        .await?
        .ok_or(RebornCompositionError::MissingSecretMasterKey)?;
    SecretCredentialStores::from_master_key(scoped_filesystem, master_key)
}

pub(crate) async fn resolve_explicit_or_keychain_master_key(
    explicit: Option<ironclaw_secrets::SecretMaterial>,
) -> Result<Option<ironclaw_secrets::SecretMaterial>, ironclaw_secrets::SecretError> {
    if let Some(master_key) = explicit {
        Ok(Some(master_key))
    } else {
        ironclaw_secrets::keychain::resolve_master_key_material().await
    }
}

pub(crate) async fn trigger_repository_for_durable_backend(
    backend: &DurableBackend,
) -> Result<Arc<dyn TriggerRepository>, RebornBuildError> {
    match backend {
        DurableBackend::LibSql { runtime, .. } => {
            let repository =
                ironclaw_triggers::LibSqlTriggerRepository::from_runtime(Arc::clone(runtime));
            repository
                .run_migrations()
                .await
                .map_err(|error| RebornBuildError::InvalidConfig {
                    reason: format!("standalone trigger repository migrations failed: {error}"),
                })?;
            Ok(Arc::new(repository))
        }
        DurableBackend::Postgres(pool) => {
            let repository = ironclaw_triggers::PostgresTriggerRepository::new(pool.clone());
            repository
                .run_migrations()
                .await
                .map_err(|error| RebornBuildError::InvalidConfig {
                    reason: format!("PostgreSQL trigger repository migrations failed: {error}"),
                })?;
            Ok(Arc::new(repository))
        }
    }
}

/// Single source for the resource-governor recipe every substrate build path
/// uses: a `FilesystemResourceGovernor` over the invocation-scoped view of the
/// composed root filesystem.
pub(crate) fn filesystem_resource_governor<F>(filesystem: &Arc<F>) -> FilesystemResourceGovernor<F>
where
    F: RootFilesystem + 'static,
{
    FilesystemResourceGovernor::new(crate::wrap_scoped(Arc::clone(filesystem)))
}

/// Validated durable stores required before upper runtime assembly begins.
pub(crate) struct ProductionStoreBundle {
    pub(crate) filesystem: Arc<CompositeRootFilesystem>,
    /// Filesystem the process journal writes through. Defaults to the
    /// data-plane `filesystem`; a Postgres deployment replaces it with one over
    /// a dedicated connection pool so heartbeats never queue behind data-plane
    /// traffic (`process_journal_root_filesystem`).
    pub(crate) process_journal_filesystem: Arc<CompositeRootFilesystem>,
    pub(crate) scoped_filesystem: Arc<ScopedFilesystem<CompositeRootFilesystem>>,
    pub(crate) resource_governor: ComposedResourceGovernor,
    pub(crate) leases: Arc<ComposedCapabilityLeaseStore>,
    pub(crate) persistent_approval_policies: Arc<ComposedPersistentApprovalPolicyStore>,
    pub(crate) secret_credentials: SecretCredentialStores<CompositeRootFilesystem>,
    pub(crate) event_store: ironclaw_event_store::RebornEventStoreConfig,
}

impl ProductionStoreBundle {
    pub(crate) async fn new(
        filesystem: Arc<CompositeRootFilesystem>,
        resource_governor: ComposedResourceGovernor,
        secret_master_key: ironclaw_secrets::SecretMaterial,
        event_store: ironclaw_event_store::RebornEventStoreConfig,
    ) -> Result<Self, RebornBuildError> {
        validate_reborn_runtime_storage(&filesystem).await?;
        let scoped_filesystem = crate::wrap_scoped(Arc::clone(&filesystem));
        let secret_credentials = SecretCredentialStores::from_master_key(
            Arc::clone(&scoped_filesystem),
            secret_master_key,
        )?;
        Self::assemble_validated(
            filesystem,
            scoped_filesystem,
            resource_governor,
            secret_credentials,
            event_store,
        )
        .await
    }

    pub(crate) async fn with_secret_credentials(
        filesystem: Arc<CompositeRootFilesystem>,
        resource_governor: ComposedResourceGovernor,
        secret_credentials: SecretCredentialStores<CompositeRootFilesystem>,
        event_store: ironclaw_event_store::RebornEventStoreConfig,
    ) -> Result<Self, RebornBuildError> {
        validate_reborn_runtime_storage(&filesystem).await?;
        let scoped_filesystem = crate::wrap_scoped(Arc::clone(&filesystem));
        Self::assemble_validated(
            filesystem,
            scoped_filesystem,
            resource_governor,
            secret_credentials,
            event_store,
        )
        .await
    }

    async fn assemble_validated(
        filesystem: Arc<CompositeRootFilesystem>,
        scoped_filesystem: Arc<ScopedFilesystem<CompositeRootFilesystem>>,
        resource_governor: ComposedResourceGovernor,
        secret_credentials: SecretCredentialStores<CompositeRootFilesystem>,
        event_store: ironclaw_event_store::RebornEventStoreConfig,
    ) -> Result<Self, RebornBuildError> {
        let leases = Arc::new(CapabilityLeaseStore::new(Arc::clone(&scoped_filesystem)));
        let persistent_approval_policies = Arc::new(PersistentApprovalPolicyStore::new(
            Arc::clone(&scoped_filesystem),
        ));
        let resource_governor = warm_resource_governor(resource_governor).await?;

        Ok(Self {
            process_journal_filesystem: Arc::clone(&filesystem),
            filesystem,
            scoped_filesystem,
            resource_governor,
            leases,
            persistent_approval_policies,
            secret_credentials,
            event_store,
        })
    }
}

impl ProductionStoreBundle {
    /// Route the process journal through its own filesystem handle. Callers that
    /// have no separate handle leave the shared one in place.
    pub(crate) fn with_process_journal_filesystem(
        mut self,
        filesystem: Option<Arc<CompositeRootFilesystem>>,
    ) -> Self {
        if let Some(filesystem) = filesystem {
            self.process_journal_filesystem = filesystem;
        }
        self
    }
}

async fn warm_resource_governor<F>(
    resource_governor: FilesystemResourceGovernor<F>,
) -> Result<FilesystemResourceGovernor<F>, RebornBuildError>
where
    F: RootFilesystem + 'static,
{
    tokio::task::spawn_blocking(move || {
        resource_governor.warm_authority()?;
        Ok::<_, ironclaw_resources::ResourceError>(resource_governor)
    })
    .await
    .map_err(|error| RebornBuildError::InvalidConfig {
        reason: format!("resource governor warm-up task failed: {error}"),
    })?
    .map_err(RebornBuildError::from)
}
