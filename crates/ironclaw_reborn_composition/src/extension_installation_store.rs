use async_trait::async_trait;
use ironclaw_extensions::{
    ExtensionActivationState, ExtensionHealthSnapshot, ExtensionInstallation,
    ExtensionInstallationError, ExtensionInstallationId, ExtensionInstallationStore,
    ExtensionManifestRecord, InMemoryExtensionInstallationStore, ManifestHash, ManifestSource,
};
use ironclaw_filesystem::{FilesystemError, RootFilesystem};
use ironclaw_host_api::{ExtensionId, VirtualPath};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

const DEFAULT_INSTALLATION_STATE_PATH: &str = "/system/extensions/.installations/state.json";

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
                state.load_into(&inner).await?;
            }
            Err(FilesystemError::NotFound { .. }) => {}
            Err(error) => {
                tracing::debug!(
                    ?error,
                    state_path = %state_path.as_str(),
                    "extension installation state load failed"
                );
                return Err(invalid_installation_error(
                    "failed to load extension installation state",
                ));
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
        let bytes = serde_json::to_vec_pretty(&state).map_err(invalid_installation_error)?;
        self.filesystem
            .write_file(&self.state_path, &bytes)
            .await
            .map_err(invalid_installation_error)
    }
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

#[derive(Debug, Default, Serialize, Deserialize)]
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
        self,
        store: &InMemoryExtensionInstallationStore,
    ) -> Result<(), ExtensionInstallationError> {
        for manifest in self.manifests {
            store
                .upsert_manifest(manifest.into_manifest_record()?)
                .await?;
        }
        for installation in self.installations {
            store.upsert_installation(installation).await?;
        }
        Ok(())
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct WireManifestRecord {
    raw_toml: String,
    source: WireManifestSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    manifest_hash: Option<ManifestHash>,
}

impl WireManifestRecord {
    fn into_manifest_record(self) -> Result<ExtensionManifestRecord, ExtensionInstallationError> {
        let host_ports = ironclaw_host_runtime::default_host_port_catalog()
            .map_err(invalid_installation_error)?;
        let contracts = ironclaw_host_runtime::default_host_api_contract_registry()
            .map_err(invalid_installation_error)?;
        ExtensionManifestRecord::from_toml_with_contracts(
            self.raw_toml,
            self.source.into_manifest_source(),
            &host_ports,
            self.manifest_hash,
            &contracts,
        )
    }
}

impl From<ExtensionManifestRecord> for WireManifestRecord {
    fn from(record: ExtensionManifestRecord) -> Self {
        Self {
            raw_toml: record.raw_toml().to_string(),
            source: WireManifestSource::from_manifest_source(record.manifest().source),
            manifest_hash: record.manifest_hash().cloned(),
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
mod tests {
    use std::sync::Arc;

    use chrono::Utc;
    use ironclaw_extensions::{
        ExtensionActivationState, ExtensionInstallationId, ExtensionManifestRecord,
        ExtensionManifestRef, MANIFEST_SCHEMA_VERSION,
    };
    use ironclaw_filesystem::{
        BackendCapabilities, CasExpectation, DirEntry, Entry, FileStat, FilesystemOperation,
        InMemoryBackend, RecordVersion, RootFilesystem, VersionedEntry,
    };
    use ironclaw_host_api::HostPortCatalog;

    use super::*;

    #[tokio::test]
    async fn load_at_treats_not_found_as_empty_state() {
        let filesystem: Arc<dyn RootFilesystem> = Arc::new(InMemoryBackend::new());
        let state_path =
            VirtualPath::new("/tenants/acme/system/extensions/.installations/missing-state.json")
                .expect("valid state path");

        let store = FilesystemExtensionInstallationStore::load_at(filesystem, state_path)
            .await
            .expect("missing state file loads as empty");

        assert!(
            store
                .list_installations()
                .await
                .expect("list installations")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn load_at_sanitizes_filesystem_read_errors() {
        let filesystem: Arc<dyn RootFilesystem> = Arc::new(ReadFailureFilesystem::new());
        let state_path =
            VirtualPath::new("/tenants/acme/system/extensions/.installations/state.json")
                .expect("valid state path");

        let error =
            match FilesystemExtensionInstallationStore::load_at(filesystem, state_path).await {
                Ok(_) => panic!("backend read failure should surface as invalid installation"),
                Err(error) => error,
            };

        let rendered = error.to_string();
        assert!(rendered.contains("failed to load extension installation state"));
        assert!(!rendered.contains("/tenants/acme"));
        assert!(!rendered.contains("raw backend detail"));
    }

    #[tokio::test]
    async fn load_at_persists_state_to_custom_path() {
        let filesystem: Arc<dyn RootFilesystem> = Arc::new(InMemoryBackend::new());
        let state_path =
            VirtualPath::new("/tenants/acme/system/extensions/.installations/state.json")
                .expect("valid state path");
        let store = FilesystemExtensionInstallationStore::load_at(
            Arc::clone(&filesystem),
            state_path.clone(),
        )
        .await
        .expect("store loads");
        let installation_id =
            ExtensionInstallationId::new("gmail".to_string()).expect("valid installation id");
        let extension_id = ExtensionId::new("gmail").expect("valid extension id");
        let manifest_ref = ExtensionManifestRef::new(extension_id.clone(), None);
        let manifest = ExtensionManifestRecord::from_toml(
            format!(
                r#"
schema_version = "{schema}"
id = "gmail"
name = "Gmail"
version = "0.1.0"
description = "test"
trust = "third_party"

[runtime]
kind = "wasm"
module = "wasm/gmail.wasm"

[[capabilities]]
id = "gmail.echo"
description = "Echoes input"
default_permission = "allow"
visibility = "model"
input_schema_ref = "schemas/gmail/echo.input.v1.json"
output_schema_ref = "schemas/gmail/echo.output.v1.json"
prompt_doc_ref = "prompts/gmail/echo.md"
"#,
                schema = MANIFEST_SCHEMA_VERSION,
            ),
            ManifestSource::HostBundled,
            &HostPortCatalog::empty(),
            None,
        )
        .expect("valid manifest");
        store
            .upsert_manifest_and_installation(
                manifest,
                ExtensionInstallation::new(
                    installation_id.clone(),
                    extension_id,
                    ExtensionActivationState::Installed,
                    manifest_ref,
                    Vec::new(),
                    Utc::now(),
                )
                .expect("valid installation"),
            )
            .await
            .expect("installation saved");

        assert!(
            filesystem
                .read_file(&state_path)
                .await
                .expect("state file exists")
                .starts_with(b"{")
        );

        let reloaded = FilesystemExtensionInstallationStore::load_at(filesystem, state_path)
            .await
            .expect("store reloads");
        assert!(
            reloaded
                .get_installation(&installation_id)
                .await
                .expect("installation read")
                .is_some()
        );
    }

    struct ReadFailureFilesystem {
        inner: InMemoryBackend,
    }

    impl ReadFailureFilesystem {
        fn new() -> Self {
            Self {
                inner: InMemoryBackend::new(),
            }
        }
    }

    #[async_trait]
    impl RootFilesystem for ReadFailureFilesystem {
        fn capabilities(&self) -> BackendCapabilities {
            self.inner.capabilities()
        }

        async fn put(
            &self,
            path: &VirtualPath,
            entry: Entry,
            cas: CasExpectation,
        ) -> Result<RecordVersion, FilesystemError> {
            self.inner.put(path, entry, cas).await
        }

        async fn get(&self, path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
            Err(FilesystemError::Backend {
                path: path.clone(),
                operation: FilesystemOperation::ReadFile,
                reason: "raw backend detail".to_string(),
            })
        }

        async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
            self.inner.list_dir(path).await
        }

        async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
            self.inner.stat(path).await
        }

        async fn delete(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
            self.inner.delete(path).await
        }
    }
}
