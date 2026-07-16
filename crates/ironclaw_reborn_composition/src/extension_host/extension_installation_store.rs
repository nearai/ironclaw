use async_trait::async_trait;
use ironclaw_extensions::{
    ExtensionActivationState, ExtensionHealthSnapshot, ExtensionInstallation,
    ExtensionInstallationError, ExtensionInstallationId, ExtensionInstallationStore,
    ExtensionManifestRecord, ExtensionRemovalCleanupRequirement,
    InMemoryExtensionInstallationStore, ManifestHash, ManifestSource,
    canonicalize_installation_rows,
};
use ironclaw_filesystem::{FilesystemError, RootFilesystem};
use ironclaw_host_api::{ExtensionId, VirtualPath};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::extension_host::host_api_contracts::product_extension_host_api_contract_registry;

const DEFAULT_INSTALLATION_STATE_PATH: &str = "/system/extensions/.installations/state.json";
const INSTALLATION_STATE_IO_ERROR: &str = "failed to load extension installation state";

pub(crate) struct FilesystemExtensionInstallationStore {
    filesystem: std::sync::Arc<dyn RootFilesystem>,
    state_path: VirtualPath,
    inner: InMemoryExtensionInstallationStore,
    /// The exact snapshot bytes this store last observed on disk (`None`
    /// before the first write). Serializes writers in-process and backs the
    /// stale-writer guard in [`Self::ensure_snapshot_current`].
    persisted_snapshot: Mutex<Option<Vec<u8>>>,
}

impl FilesystemExtensionInstallationStore {
    pub(crate) async fn load_at(
        filesystem: std::sync::Arc<dyn RootFilesystem>,
        state_path: VirtualPath,
    ) -> Result<Self, ExtensionInstallationError> {
        let inner = InMemoryExtensionInstallationStore::default();
        let persisted = match filesystem.read_file(&state_path).await {
            Ok(bytes) => {
                let state: WireState =
                    serde_json::from_slice(&bytes).map_err(invalid_installation_error)?;
                let original_installations = state.installations;
                let normalized_installations =
                    canonicalize_installation_rows(original_installations.clone())?;
                let needs_rewrite = normalized_installations != original_installations;
                let normalized_state = WireState {
                    manifests: state.manifests,
                    installations: normalized_installations,
                };

                // Validate the complete normalized snapshot before writing it
                // back. A malformed manifest/installation pair must leave the
                // persisted bytes untouched and must not expose a half-loaded
                // store.
                normalized_state.load_into(&inner).await?;
                if needs_rewrite {
                    Some(write_snapshot(&filesystem, &state_path, &normalized_state).await?)
                } else {
                    Some(bytes)
                }
            }
            Err(FilesystemError::NotFound { .. }) => None,
            Err(error) => {
                tracing::debug!(
                    ?error,
                    state_path = %state_path.as_str(),
                    "extension installation state load failed"
                );
                return Err(store_unavailable_error(INSTALLATION_STATE_IO_ERROR));
            }
        };
        Ok(Self {
            filesystem,
            state_path,
            inner,
            persisted_snapshot: Mutex::new(persisted),
        })
    }

    pub(crate) fn default_state_path() -> Result<VirtualPath, ExtensionInstallationError> {
        default_installation_state_path()
    }

    /// Stale-writer guard: the on-disk snapshot must still be exactly the
    /// bytes this store last observed, or the pending full-snapshot rewrite
    /// would silently revert a concurrent writer's state.
    ///
    /// The extension mount is served by the byte-oriented backend family
    /// (`LocalFilesystem` for materialized bundles), which cannot honor
    /// `CasExpectation::Version` — a true CAS fence is not expressible here
    /// until this store migrates onto the versioned record plane (see
    /// docs/plans/2026-06-25-cas-migration.md). Until then this read-back
    /// compare converts the split-brain failure mode (two processes over one
    /// state file, last-writer-wins) into a loud retryable error. The
    /// read-compare-write sequence is not atomic across processes; the
    /// deployment assumption stays a single serving process, and this guard
    /// makes a violation of that assumption detectable instead of silent.
    async fn ensure_snapshot_current(
        &self,
        persisted: &Option<Vec<u8>>,
    ) -> Result<(), ExtensionInstallationError> {
        let on_disk = match self.filesystem.read_file(&self.state_path).await {
            Ok(bytes) => Some(bytes),
            Err(FilesystemError::NotFound { .. }) => None,
            Err(error) => {
                tracing::debug!(
                    ?error,
                    state_path = %self.state_path.as_str(),
                    "extension installation state pre-write read failed"
                );
                return Err(store_unavailable_error(INSTALLATION_STATE_IO_ERROR));
            }
        };
        if &on_disk != persisted {
            return Err(store_unavailable_error(
                "extension installation state changed on disk under this process; \
                 another writer owns the snapshot — restart to reload it",
            ));
        }
        Ok(())
    }

    async fn save_snapshot(
        &self,
        persisted: &mut Option<Vec<u8>>,
    ) -> Result<(), ExtensionInstallationError> {
        let state = WireState::from_store(&self.inner).await?;
        let bytes = write_snapshot(&self.filesystem, &self.state_path, &state).await?;
        *persisted = Some(bytes);
        Ok(())
    }

    /// Undo an in-memory mutation whose snapshot write failed, so the
    /// in-memory view stays identical to disk and a retry replays cleanly
    /// instead of double-applying (e.g. re-deleting an already-deleted row
    /// and turning a transient IO failure into a malformed-request error).
    /// Best-effort: a rollback failure is logged and the original write
    /// error still surfaces; the store is then diverged either way and the
    /// retryable `StoreUnavailable` tells the caller to back off.
    async fn restore_installation_row(&self, prior: Option<ExtensionInstallation>) {
        let result = match prior {
            Some(prior) => {
                let installation_id = prior.installation_id().clone();
                self.inner
                    .upsert_installation(prior)
                    .await
                    .map_err(|error| (format!("restore installation {installation_id}"), error))
            }
            None => Ok(()),
        };
        if let Err((operation, error)) = result {
            tracing::debug!(
                ?error,
                operation,
                "in-memory rollback after failed snapshot write failed"
            );
        }
    }

    async fn restore_manifest_row(&self, prior: Option<ExtensionManifestRecord>) {
        let result = match prior {
            Some(prior) => {
                let extension_id = prior.extension_id().clone();
                self.inner
                    .upsert_manifest(prior)
                    .await
                    .map_err(|error| (format!("restore manifest {extension_id}"), error))
            }
            None => Ok(()),
        };
        if let Err((operation, error)) = result {
            tracing::debug!(
                ?error,
                operation,
                "in-memory rollback after failed snapshot write failed"
            );
        }
    }

    async fn remove_inserted_installation_row(&self, installation_id: &ExtensionInstallationId) {
        if let Err(error) = self.inner.delete_installation(installation_id).await {
            tracing::debug!(
                ?error,
                installation_id = %installation_id,
                "in-memory rollback after failed snapshot write failed"
            );
        }
    }

    async fn remove_inserted_manifest_row(&self, extension_id: &ExtensionId) {
        if let Err(error) = self.inner.delete_manifest(extension_id).await {
            tracing::debug!(
                ?error,
                extension_id = %extension_id,
                "in-memory rollback after failed snapshot write failed"
            );
        }
    }
}

async fn write_snapshot(
    filesystem: &std::sync::Arc<dyn RootFilesystem>,
    state_path: &VirtualPath,
    state: &WireState,
) -> Result<Vec<u8>, ExtensionInstallationError> {
    let bytes = serde_json::to_vec_pretty(state).map_err(invalid_installation_error)?;
    filesystem
        .write_file(state_path, &bytes)
        .await
        .map_err(|error| {
            tracing::debug!(
                ?error,
                state_path = %state_path.as_str(),
                "extension installation state write failed"
            );
            store_unavailable_error(INSTALLATION_STATE_IO_ERROR)
        })?;
    Ok(bytes)
}

fn default_installation_state_path() -> Result<VirtualPath, ExtensionInstallationError> {
    VirtualPath::new(DEFAULT_INSTALLATION_STATE_PATH).map_err(|error| {
        ExtensionInstallationError::InvalidInstallation {
            reason: error.to_string(),
        }
    })
}

#[async_trait]
impl ExtensionInstallationStore for FilesystemExtensionInstallationStore {
    async fn list_manifests(
        &self,
    ) -> Result<Vec<ExtensionManifestRecord>, ExtensionInstallationError> {
        self.inner.list_manifests().await
    }

    async fn get_manifest(
        &self,
        extension_id: &ExtensionId,
    ) -> Result<Option<ExtensionManifestRecord>, ExtensionInstallationError> {
        self.inner.get_manifest(extension_id).await
    }

    async fn upsert_manifest(
        &self,
        manifest: ExtensionManifestRecord,
    ) -> Result<(), ExtensionInstallationError> {
        let mut persisted = self.persisted_snapshot.lock().await;
        self.ensure_snapshot_current(&persisted).await?;
        let extension_id = manifest.extension_id().clone();
        let prior = self.inner.get_manifest(&extension_id).await?;
        self.inner.upsert_manifest(manifest).await?;
        if let Err(error) = self.save_snapshot(&mut persisted).await {
            match prior {
                Some(prior) => self.restore_manifest_row(Some(prior)).await,
                None => self.remove_inserted_manifest_row(&extension_id).await,
            }
            return Err(error);
        }
        Ok(())
    }

    async fn upsert_manifest_and_installation(
        &self,
        manifest: ExtensionManifestRecord,
        installation: ExtensionInstallation,
    ) -> Result<(), ExtensionInstallationError> {
        let mut persisted = self.persisted_snapshot.lock().await;
        self.ensure_snapshot_current(&persisted).await?;
        let extension_id = manifest.extension_id().clone();
        let installation_id = installation.installation_id().clone();
        let prior_manifest = self.inner.get_manifest(&extension_id).await?;
        let prior_installation = self.inner.get_installation(&installation_id).await?;
        self.inner
            .upsert_manifest_and_installation(manifest, installation)
            .await?;
        if let Err(error) = self.save_snapshot(&mut persisted).await {
            match prior_installation {
                Some(prior) => self.restore_installation_row(Some(prior)).await,
                None => {
                    self.remove_inserted_installation_row(&installation_id)
                        .await
                }
            }
            match prior_manifest {
                Some(prior) => self.restore_manifest_row(Some(prior)).await,
                None => self.remove_inserted_manifest_row(&extension_id).await,
            }
            return Err(error);
        }
        Ok(())
    }

    async fn list_installations(
        &self,
    ) -> Result<Vec<ExtensionInstallation>, ExtensionInstallationError> {
        self.inner.list_installations().await
    }

    async fn list_enabled_installations(
        &self,
    ) -> Result<Vec<ExtensionInstallation>, ExtensionInstallationError> {
        self.inner.list_enabled_installations().await
    }

    async fn get_installation(
        &self,
        installation_id: &ExtensionInstallationId,
    ) -> Result<Option<ExtensionInstallation>, ExtensionInstallationError> {
        self.inner.get_installation(installation_id).await
    }

    async fn upsert_installation(
        &self,
        installation: ExtensionInstallation,
    ) -> Result<(), ExtensionInstallationError> {
        let mut persisted = self.persisted_snapshot.lock().await;
        self.ensure_snapshot_current(&persisted).await?;
        let installation_id = installation.installation_id().clone();
        let prior = self.inner.get_installation(&installation_id).await?;
        self.inner.upsert_installation(installation).await?;
        if let Err(error) = self.save_snapshot(&mut persisted).await {
            match prior {
                Some(prior) => self.restore_installation_row(Some(prior)).await,
                None => {
                    self.remove_inserted_installation_row(&installation_id)
                        .await
                }
            }
            return Err(error);
        }
        Ok(())
    }

    async fn set_activation_state(
        &self,
        installation_id: &ExtensionInstallationId,
        state: ExtensionActivationState,
    ) -> Result<(), ExtensionInstallationError> {
        let mut persisted = self.persisted_snapshot.lock().await;
        self.ensure_snapshot_current(&persisted).await?;
        let prior = self.inner.get_installation(installation_id).await?;
        self.inner
            .set_activation_state(installation_id, state)
            .await?;
        if let Err(error) = self.save_snapshot(&mut persisted).await {
            self.restore_installation_row(prior).await;
            return Err(error);
        }
        Ok(())
    }

    async fn delete_installation(
        &self,
        installation_id: &ExtensionInstallationId,
    ) -> Result<(), ExtensionInstallationError> {
        let mut persisted = self.persisted_snapshot.lock().await;
        self.ensure_snapshot_current(&persisted).await?;
        let prior = self.inner.get_installation(installation_id).await?;
        self.inner.delete_installation(installation_id).await?;
        if let Err(error) = self.save_snapshot(&mut persisted).await {
            self.restore_installation_row(prior).await;
            return Err(error);
        }
        Ok(())
    }

    async fn delete_manifest(
        &self,
        extension_id: &ExtensionId,
    ) -> Result<(), ExtensionInstallationError> {
        let mut persisted = self.persisted_snapshot.lock().await;
        self.ensure_snapshot_current(&persisted).await?;
        let prior = self.inner.get_manifest(extension_id).await?;
        self.inner.delete_manifest(extension_id).await?;
        if let Err(error) = self.save_snapshot(&mut persisted).await {
            self.restore_manifest_row(prior).await;
            return Err(error);
        }
        Ok(())
    }

    async fn update_health(
        &self,
        installation_id: &ExtensionInstallationId,
        health: ExtensionHealthSnapshot,
    ) -> Result<(), ExtensionInstallationError> {
        let mut persisted = self.persisted_snapshot.lock().await;
        self.ensure_snapshot_current(&persisted).await?;
        let prior = self.inner.get_installation(installation_id).await?;
        self.inner.update_health(installation_id, health).await?;
        if let Err(error) = self.save_snapshot(&mut persisted).await {
            self.restore_installation_row(prior).await;
            return Err(error);
        }
        Ok(())
    }
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct WireState {
    manifests: Vec<WireManifestRecord>,
    installations: Vec<ExtensionInstallation>,
}

impl WireState {
    async fn from_store(
        store: &InMemoryExtensionInstallationStore,
    ) -> Result<Self, ExtensionInstallationError> {
        let manifests = store
            .list_manifests()
            .await?
            .into_iter()
            .map(WireManifestRecord::from)
            .collect();
        let installations = store.list_installations().await?;
        Ok(Self {
            manifests,
            installations,
        })
    }

    async fn load_into(
        &self,
        store: &InMemoryExtensionInstallationStore,
    ) -> Result<(), ExtensionInstallationError> {
        for manifest in &self.manifests {
            store
                .upsert_manifest(manifest.clone().into_manifest_record()?)
                .await?;
        }
        for installation in &self.installations {
            store.upsert_installation(installation.clone()).await?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WireManifestRecord {
    raw_toml: String,
    source: WireManifestSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    manifest_hash: Option<ManifestHash>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    removal_cleanup_requirements: Vec<ExtensionRemovalCleanupRequirement>,
}

impl WireManifestRecord {
    fn into_manifest_record(self) -> Result<ExtensionManifestRecord, ExtensionInstallationError> {
        let host_ports = ironclaw_host_runtime::default_host_port_catalog()
            .map_err(invalid_installation_error)?;
        let contracts =
            product_extension_host_api_contract_registry().map_err(invalid_installation_error)?;
        ExtensionManifestRecord::from_toml_with_contracts(
            self.raw_toml,
            self.source.into_manifest_source(),
            &host_ports,
            self.manifest_hash,
            &contracts,
        )
        .map(|record| record.with_removal_cleanup_requirements(self.removal_cleanup_requirements))
    }
}

impl From<ExtensionManifestRecord> for WireManifestRecord {
    fn from(record: ExtensionManifestRecord) -> Self {
        Self {
            raw_toml: record.raw_toml().to_string(),
            source: WireManifestSource::from_manifest_source(record.manifest().source),
            manifest_hash: record.manifest_hash().cloned(),
            removal_cleanup_requirements: record.removal_cleanup_requirements().to_vec(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum WireManifestSource {
    HostBundled,
    InstalledLocal,
    RegistryInstalled,
}

impl WireManifestSource {
    fn from_manifest_source(source: ManifestSource) -> Self {
        match source {
            ManifestSource::HostBundled => Self::HostBundled,
            ManifestSource::InstalledLocal => Self::InstalledLocal,
            ManifestSource::RegistryInstalled => Self::RegistryInstalled,
        }
    }

    fn into_manifest_source(self) -> ManifestSource {
        match self {
            Self::HostBundled => ManifestSource::HostBundled,
            Self::InstalledLocal => ManifestSource::InstalledLocal,
            Self::RegistryInstalled => ManifestSource::RegistryInstalled,
        }
    }
}

fn invalid_installation_error(error: impl std::fmt::Display) -> ExtensionInstallationError {
    ExtensionInstallationError::InvalidInstallation {
        reason: error.to_string(),
    }
}

fn store_unavailable_error(error: impl std::fmt::Display) -> ExtensionInstallationError {
    ExtensionInstallationError::StoreUnavailable {
        reason: error.to_string(),
    }
}

#[cfg(test)]
mod tests;
