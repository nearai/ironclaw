use async_trait::async_trait;
use ironclaw_extensions::{
    ExtensionActivationState, ExtensionHealthSnapshot, ExtensionInstallation,
    ExtensionInstallationError, ExtensionInstallationId, ExtensionInstallationStore,
    ExtensionManifestRecord, ExtensionManifestRef, InMemoryExtensionInstallationStore,
    ManifestHash, ManifestSource,
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
                let mut state: WireState =
                    serde_json::from_slice(&bytes).map_err(invalid_installation_error)?;
                let migrated = migrate_retired_slack_bot_identity(&mut state);
                state.load_into(&inner).await?;
                if migrated {
                    tracing::debug!(
                        "migrated retired slack_bot installation state to the unified slack extension"
                    );
                    let store = Self {
                        filesystem,
                        state_path,
                        inner,
                        save_lock: Mutex::new(()),
                    };
                    store.save_snapshot().await?;
                    return Ok(store);
                }
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

/// One-time forward migration (NEA-25 unified Slack extension): the Slack
/// channel package identity `slack_bot` merged into the unified `slack`
/// extension. Persisted state written by earlier builds may still carry
/// `slack_bot` manifest records and installation rows; fold them forward so
/// the store only ever holds the unified identity. The `slack_bot` manifest
/// record is dropped (the unified manifest ships host-bundled). Its
/// installation state merges into `slack`'s: an enabled `slack_bot` install
/// keeps the unified extension enabled, and credential bindings union. This
/// is a load-time data migration persisted immediately — no code path
/// resolves the retired identity afterwards.
fn migrate_retired_slack_bot_identity(state: &mut WireState) -> bool {
    let manifest_count = state.manifests.len();
    state
        .manifests
        .retain(|record| !record.raw_toml.contains("\nid = \"slack_bot\""));
    let mut changed = state.manifests.len() != manifest_count;

    let retired: Vec<ExtensionInstallation> = state
        .installations
        .iter()
        .filter(|installation| installation.extension_id().as_str() == "slack_bot")
        .cloned()
        .collect();
    if retired.is_empty() {
        return changed;
    }
    state
        .installations
        .retain(|installation| installation.extension_id().as_str() != "slack_bot");
    changed = true;

    let retired_enabled = retired
        .iter()
        .any(|installation| installation.activation_state() == ExtensionActivationState::Enabled);
    let retired_bindings: Vec<_> = retired
        .iter()
        .flat_map(|installation| installation.credential_bindings().iter().cloned())
        .collect();

    let Ok(unified_id) = ExtensionId::new("slack") else {
        return changed;
    };
    // The store fails closed on installations without a matching manifest
    // record. If the legacy state only ever installed the bot channel, no
    // `slack` record exists yet — seed the host-bundled unified manifest so
    // the folded installation stays loadable.
    let has_unified_record = state
        .manifests
        .iter()
        .any(|record| record.raw_toml.contains("\nid = \"slack\""));
    if !has_unified_record {
        #[cfg(feature = "slack-v2-host-beta")]
        {
            state.manifests.push(WireManifestRecord {
                raw_toml: super::available_extensions::slack_manifest_toml().to_string(),
                source: WireManifestSource::HostBundled,
                manifest_hash: None,
            });
        }
        #[cfg(not(feature = "slack-v2-host-beta"))]
        {
            // Without the Slack feature the unified manifest is not bundled;
            // drop the orphaned installation instead of failing the load.
            return changed;
        }
    }
    if let Some(existing) = state
        .installations
        .iter_mut()
        .find(|installation| installation.extension_id() == &unified_id)
    {
        let activation = if retired_enabled {
            ExtensionActivationState::Enabled
        } else {
            existing.activation_state()
        };
        let mut bindings = existing.credential_bindings().to_vec();
        for binding in retired_bindings {
            if !bindings.contains(&binding) {
                bindings.push(binding);
            }
        }
        if let Ok(merged) = ExtensionInstallation::new(
            existing.installation_id().clone(),
            unified_id.clone(),
            activation,
            ExtensionManifestRef::new(unified_id, None),
            bindings,
            chrono::Utc::now(),
        ) {
            *existing = merged;
        }
    } else if let Some(first) = retired.into_iter().next()
        && let Ok(renamed) = ExtensionInstallation::new(
            first.installation_id().clone(),
            unified_id.clone(),
            first.activation_state(),
            ExtensionManifestRef::new(unified_id, None),
            first.credential_bindings().to_vec(),
            chrono::Utc::now(),
        )
    {
        state.installations.push(renamed);
    }
    changed
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
        ExtensionManifestRecord::from_toml(
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

    fn capability_provider_contracts() -> ironclaw_extensions::HostApiContractRegistry {
        let mut contracts = ironclaw_extensions::HostApiContractRegistry::new();
        contracts
            .register(std::sync::Arc::new(
                ironclaw_extensions::CapabilityProviderHostApiContract::new()
                    .expect("capability provider contract"),
            ))
            .expect("register capability provider contract");
        contracts
    }
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
    async fn load_at_migrates_retired_slack_bot_identity_forward() {
        // One-time forward migration: persisted state from the split-identity
        // era carries slack_bot installation rows. Loading folds them into
        // the unified slack extension and persists immediately, so no code
        // path ever resolves the retired identity.
        let filesystem: Arc<dyn RootFilesystem> = Arc::new(InMemoryBackend::new());
        let state_path =
            VirtualPath::new("/tenants/acme/system/extensions/.installations/state.json")
                .expect("valid state path");

        let slack_bot_id = ExtensionId::new("slack_bot").expect("extension id");
        let slack_id = ExtensionId::new("slack").expect("extension id");
        let legacy_state = WireState {
            manifests: vec![WireManifestRecord {
                raw_toml: "schema_version = \"reborn.extension_manifest.v2\"\nid = \"slack_bot\"\n(historical split-identity record; dropped without parsing)".to_string(),
                source: WireManifestSource::HostBundled,
                manifest_hash: None,
            }],
            installations: vec![
                ExtensionInstallation::new(
                    ExtensionInstallationId::new("slack_bot".to_string())
                        .expect("installation id"),
                    slack_bot_id.clone(),
                    ExtensionActivationState::Enabled,
                    ExtensionManifestRef::new(slack_bot_id, None),
                    Vec::new(),
                    Utc::now(),
                )
                .expect("legacy installation"),
                ExtensionInstallation::new(
                    ExtensionInstallationId::new("slack".to_string()).expect("installation id"),
                    slack_id.clone(),
                    ExtensionActivationState::Installed,
                    ExtensionManifestRef::new(slack_id.clone(), None),
                    Vec::new(),
                    Utc::now(),
                )
                .expect("tools installation"),
            ],
        };
        filesystem
            .write_file(
                &state_path,
                &serde_json::to_vec(&legacy_state).expect("state serializes"),
            )
            .await
            .expect("seed legacy state");

        let store = FilesystemExtensionInstallationStore::load_at(
            Arc::clone(&filesystem),
            state_path.clone(),
        )
        .await
        .expect("store loads with migration");

        let installations = store.list_installations().await.expect("list");
        assert_eq!(installations.len(), 1, "{installations:?}");
        let unified = &installations[0];
        assert_eq!(unified.extension_id(), &slack_id);
        assert_eq!(
            unified.activation_state(),
            ExtensionActivationState::Enabled,
            "an enabled slack_bot install keeps the unified extension enabled"
        );

        // The migrated snapshot is persisted immediately: reloading from disk
        // sees only the unified identity.
        let persisted = filesystem
            .read_file(&state_path)
            .await
            .expect("read migrated snapshot");
        let rendered = String::from_utf8(persisted).expect("utf8 snapshot");
        // The unified manifest legitimately keeps the `slack_bot_token`
        // credential HANDLE; only the extension identity is retired.
        assert!(
            !rendered.contains("\"slack_bot\""),
            "migrated snapshot must not carry the retired identity: {rendered}"
        );
        assert!(
            !rendered.contains("id = \\\"slack_bot\\\""),
            "migrated snapshot must not carry the retired manifest record"
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

[[host_api]]
id = "ironclaw.capability_provider/v1"
section = "capability_provider.tools"

[capability_provider.tools]

[[capability_provider.tools.capabilities]]
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
            &capability_provider_contracts(),
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
