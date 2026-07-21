use async_trait::async_trait;
use ironclaw_extensions::{
    ExtensionActivationState, ExtensionHealthSnapshot, ExtensionInstallation,
    ExtensionInstallationError, ExtensionInstallationId, ExtensionInstallationStore,
    ExtensionManifestRecord, ExtensionRemovalCleanupRequirement,
    InMemoryExtensionInstallationStore, ManifestHash, ManifestSource, ResolvedExtensionManifest,
    canonicalize_installation_rows,
};
use ironclaw_filesystem::{FilesystemError, RootFilesystem};
use ironclaw_host_api::{ExtensionId, VirtualPath};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

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
                    channel_configs: state.channel_configs,
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

    async fn channel_config(
        &self,
        extension_id: &ExtensionId,
    ) -> Result<Vec<(String, String)>, ExtensionInstallationError> {
        self.inner.channel_config(extension_id).await
    }

    async fn set_channel_config(
        &self,
        extension_id: &ExtensionId,
        values: Vec<(String, String)>,
    ) -> Result<(), ExtensionInstallationError> {
        let mut persisted = self.persisted_snapshot.lock().await;
        self.ensure_snapshot_current(&persisted).await?;
        let prior = self.inner.channel_config(extension_id).await?;
        self.inner.set_channel_config(extension_id, values).await?;
        if let Err(error) = self.save_snapshot(&mut persisted).await {
            // Undo the in-memory mutation whose snapshot write failed, the
            // same restore-on-failure contract as the sibling mutators; a
            // failed restore leaves the next `ensure_snapshot_current` to
            // reconcile from the durable snapshot.
            let _ = self.inner.set_channel_config(extension_id, prior).await; // silent-ok: best-effort rollback, durable reload reconciles
            return Err(error);
        }
        Ok(())
    }
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct WireState {
    manifests: Vec<WireManifestRecord>,
    installations: Vec<ExtensionInstallation>,
    /// Per-installation non-secret `[channel.config]` values keyed by field
    /// handle. Serde-defaulted (and omitted when empty) so state files
    /// written before the configure surface existed load unchanged.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    channel_configs: Vec<WireChannelConfig>,
}

/// One installation's stored non-secret channel-config values.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct WireChannelConfig {
    extension_id: String,
    values: Vec<(String, String)>,
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
        let channel_configs = store
            .channel_configs()
            .await
            .into_iter()
            .map(|(extension_id, values)| WireChannelConfig {
                extension_id: extension_id.as_str().to_string(),
                values,
            })
            .collect();
        Ok(Self {
            manifests,
            installations,
            channel_configs,
        })
    }

    /// Load the wire state into the in-memory store.
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
        for config in &self.channel_configs {
            let extension_id =
                ExtensionId::new(&config.extension_id).map_err(invalid_installation_error)?;
            store
                .set_channel_config(&extension_id, config.values.clone())
                .await?;
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
    /// The compiled contract (extension-runtime REC-1/REC-2). Every record is
    /// persisted with it and loads rebuild from it without reparsing
    /// `raw_toml`. This is a blank-slate schema: there is no pre-PR record to
    /// migrate, so an absent `resolved` is a corrupt/unexpected row and the
    /// load fails loud (see `into_manifest_record`) — it is never backfilled.
    /// The field stays `Option` only so a truncated/garbled row deserializes
    /// far enough to be rejected with a clear error rather than a serde panic.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    resolved: Option<ResolvedExtensionManifest>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    removal_cleanup_requirements: Vec<ExtensionRemovalCleanupRequirement>,
}

impl WireManifestRecord {
    fn into_manifest_record(self) -> Result<ExtensionManifestRecord, ExtensionInstallationError> {
        // Every installed record is persisted with its compiled resolved
        // contract (see `From<ExtensionManifestRecord>`), so a load always
        // rebuilds from `resolved` and never reparses `raw_toml`. A record
        // without a resolved contract is not a legacy row to backfill — it is
        // corrupt or unexpected, so fail loud instead of reparsing.
        let resolved = self.resolved.ok_or_else(|| {
            invalid_installation_error(
                "installed extension record is missing its resolved manifest contract",
            )
        })?;
        Ok(ExtensionManifestRecord::from_resolved(
            self.raw_toml,
            self.source.into_manifest_source(),
            resolved,
            self.manifest_hash,
        )?
        .with_removal_cleanup_requirements(self.removal_cleanup_requirements))
    }
}

impl From<ExtensionManifestRecord> for WireManifestRecord {
    fn from(record: ExtensionManifestRecord) -> Self {
        Self {
            raw_toml: record.raw_toml().to_string(),
            source: WireManifestSource::from_manifest_source(record.manifest().source),
            manifest_hash: record.manifest_hash().cloned(),
            resolved: Some(record.resolved().clone()),
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
    use crate::extension_host::host_api_contracts::product_extension_host_api_contract_registry;

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
                    ironclaw_extensions::InstallationOwner::Tenant,
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

    const RESOLVED_RECORD_V3_MANIFEST: &str = r#"
schema_version = "reborn.extension_manifest.v3"
id = "zephyrite"
name = "Zephyrite"
version = "0.1.0"
description = "resolved-record store fixture"
trust = "third_party"

[runtime]
kind = "wasm"
module = "wasm/zephyrite_tool.wasm"

[[tools]]
id = "zephyrite.echo"
description = "Echoes input"
effects = ["network", "use_secret"]
default_permission = "ask"
visibility = "model"
input_schema_ref = "schemas/zephyrite/echo.input.v1.json"

[[tools.credentials]]
handle = "zephyrite_token"
vendor = "zephyrite"
scopes = ["echo:read"]
audience = { scheme = "https", host = "api.zephyrite.example" }
injection = { type = "header", name = "authorization", prefix = "Bearer " }

[auth.zephyrite]
method = "oauth2_code"
display_name = "Zephyrite account"
authorization_endpoint = "https://auth.zephyrite.example/authorize"
token_endpoint = "https://auth.zephyrite.example/token"
scopes = ["echo:read"]
client_credentials = { client_id_handle = "zephyrite_client_id" }

[auth.zephyrite.token_response]
access_token = "/access_token"
"#;

    fn state_path() -> VirtualPath {
        VirtualPath::new("/system/extensions/.installations/state.json").expect("state path")
    }

    /// REC-2 (store tier): a persisted record rehydrates from its resolved
    /// contract without reparsing the raw source — proven by corrupting the
    /// stored `raw_toml` to garbage and loading successfully.
    async fn assert_rehydrates_without_reparse(filesystem: Arc<dyn RootFilesystem>) {
        let store =
            FilesystemExtensionInstallationStore::load_at(Arc::clone(&filesystem), state_path())
                .await
                .expect("empty store loads");
        let record = ExtensionManifestRecord::from_toml(
            RESOLVED_RECORD_V3_MANIFEST,
            ManifestSource::InstalledLocal,
            &ironclaw_host_runtime::default_host_port_catalog().expect("catalog"),
            None,
            &product_extension_host_api_contract_registry().expect("contracts"),
        )
        .expect("v3 manifest parses");
        let expected_manifest = record.manifest().clone();
        let expected_resolved = record.resolved().clone();
        store.upsert_manifest(record).await.expect("persist record");
        drop(store);

        // Corrupt the raw source in the persisted state: the resolved
        // contract must be the load-bearing copy.
        let bytes = filesystem
            .read_file(&state_path())
            .await
            .expect("state file exists");
        let mut state: serde_json::Value =
            serde_json::from_slice(&bytes).expect("state file is JSON");
        state["manifests"][0]["raw_toml"] =
            serde_json::Value::String("# raw manifest source unavailable".to_string());
        filesystem
            .write_file(
                &state_path(),
                &serde_json::to_vec_pretty(&state).expect("serialize"),
            )
            .await
            .expect("rewrite state");

        let store =
            FilesystemExtensionInstallationStore::load_at(Arc::clone(&filesystem), state_path())
                .await
                .expect("store loads from resolved contracts");
        let reloaded = store
            .get_manifest(&ExtensionId::new("zephyrite").expect("id"))
            .await
            .expect("get manifest")
            .expect("record present");
        assert_eq!(reloaded.manifest(), &expected_manifest);
        assert_eq!(reloaded.resolved(), &expected_resolved);
        assert_eq!(reloaded.raw_toml(), "# raw manifest source unavailable");
    }

    /// Channel-config persistence (extension-runtime §6.4): values round-trip
    /// through the durable snapshot, state files written BEFORE the configure
    /// surface existed load unchanged (serde default), and a snapshot with no
    /// config keeps the old wire shape (no `channel_configs` key).
    #[tokio::test]
    async fn channel_config_round_trips_and_old_state_files_load() {
        let filesystem: Arc<dyn RootFilesystem> = Arc::new(InMemoryBackend::new());
        let store =
            FilesystemExtensionInstallationStore::load_at(Arc::clone(&filesystem), state_path())
                .await
                .expect("empty store loads");
        let record = ExtensionManifestRecord::from_toml(
            RESOLVED_RECORD_V3_MANIFEST,
            ManifestSource::InstalledLocal,
            &ironclaw_host_runtime::default_host_port_catalog().expect("catalog"),
            None,
            &product_extension_host_api_contract_registry().expect("contracts"),
        )
        .expect("v3 manifest parses");
        let extension_id = ExtensionId::new("zephyrite").expect("id");
        let installation_id =
            ExtensionInstallationId::new("zephyrite".to_string()).expect("installation id");
        store
            .upsert_manifest_and_installation(
                record,
                ExtensionInstallation::new(
                    installation_id.clone(),
                    extension_id.clone(),
                    ExtensionActivationState::Installed,
                    ExtensionManifestRef::new(extension_id.clone(), None),
                    Vec::new(),
                    Utc::now(),
                    ironclaw_extensions::InstallationOwner::Tenant,
                )
                .expect("installation"),
            )
            .await
            .expect("persist install");

        // No config saved yet: the snapshot keeps the pre-configure wire
        // shape, so this snapshot IS an "old state file" for the reload below.
        let bytes = filesystem
            .read_file(&state_path())
            .await
            .expect("state file exists");
        let state: serde_json::Value = serde_json::from_slice(&bytes).expect("state is JSON");
        assert!(
            state.get("channel_configs").is_none(),
            "an empty channel-config set must not change the wire shape"
        );
        drop(store);
        let store =
            FilesystemExtensionInstallationStore::load_at(Arc::clone(&filesystem), state_path())
                .await
                .expect("old-shape state loads");
        assert!(
            store
                .channel_config(&extension_id)
                .await
                .expect("read config")
                .is_empty()
        );

        // Save, reload from disk, and read back: the values are durable.
        let values = vec![(
            "public_endpoint_url".to_string(),
            "https://hooks.example.test/updates".to_string(),
        )];
        store
            .set_channel_config(&extension_id, values.clone())
            .await
            .expect("save config");
        drop(store);
        let store =
            FilesystemExtensionInstallationStore::load_at(Arc::clone(&filesystem), state_path())
                .await
                .expect("state with config loads");
        assert_eq!(
            store
                .channel_config(&extension_id)
                .await
                .expect("read config"),
            values
        );

        // Deleting the installation deletes its config from the snapshot too.
        store
            .delete_installation(&installation_id)
            .await
            .expect("delete installation");
        drop(store);
        let store = FilesystemExtensionInstallationStore::load_at(filesystem, state_path())
            .await
            .expect("state reloads after delete");
        assert!(
            store
                .channel_config(&extension_id)
                .await
                .expect("read config after delete")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn records_rehydrate_from_resolved_in_memory() {
        assert_rehydrates_without_reparse(Arc::new(InMemoryBackend::new())).await;
    }

    async fn libsql_filesystem(dir: &std::path::Path) -> Arc<dyn RootFilesystem> {
        let db = std::sync::Arc::new(
            libsql::Builder::new_local(dir.join("store.db"))
                .build()
                .await
                .expect("open libsql database"),
        );
        let filesystem = ironclaw_filesystem::LibSqlRootFilesystem::new(db);
        filesystem.run_migrations().await.expect("migrations");
        Arc::new(filesystem)
    }

    #[tokio::test]
    async fn records_rehydrate_from_resolved_on_libsql() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert_rehydrates_without_reparse(libsql_filesystem(dir.path()).await).await;
    }
}
