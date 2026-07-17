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
    save_lock: Mutex<()>,
}

impl FilesystemExtensionInstallationStore {
    pub(crate) async fn load_at(
        filesystem: std::sync::Arc<dyn RootFilesystem>,
        state_path: VirtualPath,
    ) -> Result<Self, ExtensionInstallationError> {
        let inner = InMemoryExtensionInstallationStore::default();
        match filesystem.read_file(&state_path).await {
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
                    write_snapshot(&filesystem, &state_path, &normalized_state).await?;
                }
            }
            Err(FilesystemError::NotFound { .. }) => {}
            Err(error) => {
                tracing::debug!(
                    ?error,
                    state_path = %state_path.as_str(),
                    "extension installation state load failed"
                );
                return Err(invalid_installation_error(INSTALLATION_STATE_IO_ERROR));
            }
        }
        Ok(Self {
            filesystem,
            state_path,
            inner,
            save_lock: Mutex::new(()),
        })
    }

    pub(crate) fn default_state_path() -> Result<VirtualPath, ExtensionInstallationError> {
        default_installation_state_path()
    }

    async fn save_snapshot(&self) -> Result<(), ExtensionInstallationError> {
        let state = WireState::from_store(&self.inner).await?;
        write_snapshot(&self.filesystem, &self.state_path, &state).await
    }
}

async fn write_snapshot(
    filesystem: &std::sync::Arc<dyn RootFilesystem>,
    state_path: &VirtualPath,
    state: &WireState,
) -> Result<(), ExtensionInstallationError> {
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
            invalid_installation_error(INSTALLATION_STATE_IO_ERROR)
        })
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
        let _guard = self.save_lock.lock().await;
        self.inner.upsert_manifest(manifest).await?;
        self.save_snapshot().await
    }

    async fn upsert_manifest_and_installation(
        &self,
        manifest: ExtensionManifestRecord,
        installation: ExtensionInstallation,
    ) -> Result<(), ExtensionInstallationError> {
        let _guard = self.save_lock.lock().await;
        self.inner
            .upsert_manifest_and_installation(manifest, installation)
            .await?;
        self.save_snapshot().await
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
        let _guard = self.save_lock.lock().await;
        self.inner.upsert_installation(installation).await?;
        self.save_snapshot().await
    }

    async fn set_activation_state(
        &self,
        installation_id: &ExtensionInstallationId,
        state: ExtensionActivationState,
    ) -> Result<(), ExtensionInstallationError> {
        let _guard = self.save_lock.lock().await;
        self.inner
            .set_activation_state(installation_id, state)
            .await?;
        self.save_snapshot().await
    }

    async fn delete_installation(
        &self,
        installation_id: &ExtensionInstallationId,
    ) -> Result<(), ExtensionInstallationError> {
        let _guard = self.save_lock.lock().await;
        self.inner.delete_installation(installation_id).await?;
        self.save_snapshot().await
    }

    async fn delete_manifest(
        &self,
        extension_id: &ExtensionId,
    ) -> Result<(), ExtensionInstallationError> {
        let _guard = self.save_lock.lock().await;
        self.inner.delete_manifest(extension_id).await?;
        self.save_snapshot().await
    }

    async fn update_health(
        &self,
        installation_id: &ExtensionInstallationId,
        health: ExtensionHealthSnapshot,
    ) -> Result<(), ExtensionInstallationError> {
        let _guard = self.save_lock.lock().await;
        self.inner.update_health(installation_id, health).await?;
        self.save_snapshot().await
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

#[cfg(test)]
mod tests;
