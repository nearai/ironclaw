use async_trait::async_trait;
use ironclaw_extensions::{
    ExtensionActivationState, ExtensionHealthSnapshot, ExtensionInstallation,
    ExtensionInstallationError, ExtensionInstallationId, ExtensionInstallationPersistedParts,
    ExtensionInstallationStore, ExtensionManifestRecord, ExtensionManifestRef,
    ExtensionRemovalCleanupRequirement, InMemoryExtensionInstallationStore,
    MANIFEST_SCHEMA_VERSION, MAX_MANIFEST_BYTES, ManifestHash, ManifestSource, ManifestV2Error,
    canonicalize_installation_rows,
};
use ironclaw_filesystem::{
    CasApply, CasUpdateError, Entry, FilesystemError, RootFilesystem, cas_update_root,
};
use ironclaw_host_api::{ExtensionId, VirtualPath};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::extension_host::host_api_contracts::product_extension_host_api_contract_registry;

const DEFAULT_INSTALLATION_STATE_PATH: &str = "/system/extensions/.installations/state.json";
const INSTALLATION_STATE_IO_ERROR: &str = "failed to load extension installation state";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NonCasLoadPolicy {
    RequireCas,
    AllowReadOnlyLocalDev,
}

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
        Self::load_at_with_policy(filesystem, state_path, NonCasLoadPolicy::RequireCas).await
    }

    pub(crate) async fn load_at_with_policy(
        filesystem: std::sync::Arc<dyn RootFilesystem>,
        state_path: VirtualPath,
        non_cas_policy: NonCasLoadPolicy,
    ) -> Result<Self, ExtensionInstallationError> {
        let inner = InMemoryExtensionInstallationStore::default();
        let state = match load_normalized_snapshot(filesystem.as_ref(), &state_path).await {
            Ok(state) => state,
            Err(CasUpdateError::Apply(error)) => return Err(error),
            Err(error @ CasUpdateError::CasUnsupported) => {
                if non_cas_policy == NonCasLoadPolicy::AllowReadOnlyLocalDev {
                    load_normalized_snapshot_without_cas(filesystem.as_ref(), &state_path).await?
                } else {
                    return Err(map_load_backend_error(&state_path, &error));
                }
            }
            Err(error) => {
                return Err(map_load_backend_error(&state_path, &error));
            }
        };
        if let Some(state) = state {
            // The CAS outcome comes from the winning snapshot. Loading only
            // after the bounded update finishes prevents a losing attempt from
            // exposing stale or partially normalized state in memory.
            state.load_into(&inner).await?;
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

async fn load_normalized_snapshot(
    filesystem: &dyn RootFilesystem,
    state_path: &VirtualPath,
) -> Result<Option<WireState>, CasUpdateError<ExtensionInstallationError>> {
    cas_update_root(
        filesystem,
        state_path,
        decode_wire_state,
        encode_wire_state,
        |current| async move {
            let Some(current) = current else {
                return Ok(CasApply::no_op(WireState::default(), None));
            };
            let normalized = normalize_wire_state(current)?;
            validate_wire_state(&normalized).await?;
            Ok(CasApply::new(normalized.clone(), Some(normalized)))
        },
    )
    .await
}

async fn load_normalized_snapshot_without_cas(
    filesystem: &dyn RootFilesystem,
    state_path: &VirtualPath,
) -> Result<Option<WireState>, ExtensionInstallationError> {
    let Some(versioned) = filesystem
        .get(state_path)
        .await
        .map_err(|error| map_filesystem_load_error(state_path, &error))?
    else {
        return Ok(None);
    };
    let current = decode_wire_state(&versioned.entry.body)?;
    let normalized = normalize_wire_state(current.clone())?;
    validate_wire_state(&normalized).await?;
    if normalized != current {
        tracing::warn!(
            state_path = %state_path.as_str(),
            "extension installation state was normalized in memory but not persisted because the filesystem backend does not support CAS"
        );
    }
    Ok(Some(normalized))
}

fn decode_wire_state(bytes: &[u8]) -> Result<WireState, ExtensionInstallationError> {
    serde_json::from_slice(bytes).map_err(invalid_installation_error)
}

fn encode_wire_state(state: &WireState) -> Result<Entry, ExtensionInstallationError> {
    serde_json::to_vec_pretty(state)
        .map(Entry::bytes)
        .map_err(invalid_installation_error)
}

async fn validate_wire_state(state: &WireState) -> Result<(), ExtensionInstallationError> {
    let candidate = InMemoryExtensionInstallationStore::default();
    state.load_into(&candidate).await
}

fn map_load_backend_error(
    state_path: &VirtualPath,
    error: &CasUpdateError<ExtensionInstallationError>,
) -> ExtensionInstallationError {
    tracing::debug!(
        ?error,
        state_path = %state_path.as_str(),
        "extension installation state CAS load failed"
    );
    invalid_installation_error(INSTALLATION_STATE_IO_ERROR)
}

fn map_filesystem_load_error(
    state_path: &VirtualPath,
    error: &FilesystemError,
) -> ExtensionInstallationError {
    tracing::debug!(
        ?error,
        state_path = %state_path.as_str(),
        "extension installation state compatibility load failed"
    );
    invalid_installation_error(INSTALLATION_STATE_IO_ERROR)
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

/// Pure, rerunnable persisted-state normalization. The transition is private
/// to this store: public manifest ingestion remains strict and continues to
/// reject the retired top-level capability shape.
fn normalize_wire_state(mut state: WireState) -> Result<WireState, ExtensionInstallationError> {
    for manifest in &mut state.manifests {
        normalize_persisted_legacy_manifest(manifest)?;
    }
    normalize_retired_slack_identity(&mut state)?;
    state.installations = canonicalize_installation_rows(state.installations)?;
    Ok(state)
}

fn normalize_persisted_legacy_manifest(
    record: &mut WireManifestRecord,
) -> Result<(), ExtensionInstallationError> {
    if !matches!(record.source, WireManifestSource::HostBundled) {
        return Ok(());
    }
    let mut document = parse_persisted_manifest_toml(&record.raw_toml)?;
    let Some(root) = document.as_table_mut() else {
        return Err(invalid_installation_error(
            "persisted extension manifest root must be a TOML table",
        ));
    };
    let Some(capabilities) = root.get("capabilities") else {
        return Ok(());
    };
    let exact_legacy_shape = root
        .get("schema_version")
        .and_then(toml::Value::as_str)
        .is_some_and(|version| version == MANIFEST_SCHEMA_VERSION)
        && capabilities
            .as_array()
            .is_some_and(|entries| !entries.is_empty())
        && !root.contains_key("host_api")
        && !root.contains_key("capability_provider");
    if !exact_legacy_shape {
        return Ok(());
    }

    let capabilities = root.remove("capabilities").ok_or_else(|| {
        invalid_installation_error("persisted legacy manifest capabilities disappeared")
    })?;
    let mut contract = toml::value::Table::new();
    contract.insert(
        "id".to_string(),
        toml::Value::String("ironclaw.capability_provider/v1".to_string()),
    );
    contract.insert(
        "section".to_string(),
        toml::Value::String("capability_provider.tools".to_string()),
    );
    root.insert(
        "host_api".to_string(),
        toml::Value::Array(vec![toml::Value::Table(contract)]),
    );
    let mut tools = toml::value::Table::new();
    tools.insert("capabilities".to_string(), capabilities);
    let mut capability_provider = toml::value::Table::new();
    capability_provider.insert("tools".to_string(), toml::Value::Table(tools));
    root.insert(
        "capability_provider".to_string(),
        toml::Value::Table(capability_provider),
    );
    record.raw_toml = toml::to_string_pretty(&document).map_err(invalid_installation_error)?;
    // Validate the converted record before any later identity fold can remove
    // it. This keeps malformed persisted input fail-closed even when the
    // record belongs to a retired extension identity.
    record.clone().into_manifest_record()?;
    Ok(())
}

fn normalize_retired_slack_identity(
    state: &mut WireState,
) -> Result<(), ExtensionInstallationError> {
    const RETIRED_SLACK_ID: &str = "slack_bot";
    const UNIFIED_SLACK_ID: &str = "slack";

    let manifest_ids = state
        .manifests
        .iter()
        .map(|record| persisted_manifest_id(&record.raw_toml))
        .collect::<Result<Vec<_>, _>>()?;
    let has_retired_state = manifest_ids.iter().any(|id| id == RETIRED_SLACK_ID)
        || state
            .installations
            .iter()
            .any(|installation| installation.extension_id().as_str() == RETIRED_SLACK_ID);
    if !has_retired_state {
        return Ok(());
    }
    if manifest_ids
        .iter()
        .zip(&state.manifests)
        .any(|(id, record)| {
            id == RETIRED_SLACK_ID && !matches!(record.source, WireManifestSource::HostBundled)
        })
    {
        return Err(invalid_installation_error(
            "retired Slack manifests must be persisted as host-bundled records",
        ));
    }

    let unified_indices = manifest_ids
        .iter()
        .enumerate()
        .filter_map(|(index, id)| (id == UNIFIED_SLACK_ID).then_some(index))
        .collect::<Vec<_>>();
    let unified_record = match unified_indices.as_slice() {
        [] => bundled_slack_wire_manifest()?,
        [index] => state.manifests[*index].clone(),
        _ => {
            return Err(invalid_installation_error(
                "persisted extension state contains multiple unified Slack manifests",
            ));
        }
    };
    // Resolve the target through the strict parser before removing any
    // retired record. Feature-disabled or malformed target state therefore
    // fails without producing a destructive candidate snapshot.
    unified_record.clone().into_manifest_record()?;
    let unified_id = ExtensionId::new(UNIFIED_SLACK_ID).map_err(invalid_installation_error)?;
    let unified_ref =
        ExtensionManifestRef::new(unified_id.clone(), unified_record.manifest_hash.clone());

    if unified_indices.is_empty() {
        state.manifests.push(unified_record);
    }
    let mut retained_manifests = Vec::with_capacity(state.manifests.len());
    for record in state.manifests.drain(..) {
        if persisted_manifest_id(&record.raw_toml)? != RETIRED_SLACK_ID {
            retained_manifests.push(record);
        }
    }
    state.manifests = retained_manifests;

    let mut installations = Vec::with_capacity(state.installations.len());
    for installation in state.installations.drain(..) {
        if installation.extension_id().as_str() == RETIRED_SLACK_ID {
            installations.push(rebuild_installation(
                &installation,
                unified_id.clone(),
                unified_ref.clone(),
                installation.activation_state(),
            )?);
        } else {
            installations.push(installation);
        }
    }
    let enabled_wins = installations.iter().any(|installation| {
        installation.extension_id() == &unified_id
            && installation.activation_state() == ExtensionActivationState::Enabled
    });
    if enabled_wins {
        for installation in &mut installations {
            if installation.extension_id() == &unified_id
                && installation.activation_state() != ExtensionActivationState::Enabled
            {
                *installation = rebuild_installation(
                    installation,
                    unified_id.clone(),
                    installation.manifest_ref().clone(),
                    ExtensionActivationState::Enabled,
                )?;
            }
        }
    }
    state.installations = installations;
    Ok(())
}

fn rebuild_installation(
    installation: &ExtensionInstallation,
    extension_id: ExtensionId,
    manifest_ref: ExtensionManifestRef,
    activation_state: ExtensionActivationState,
) -> Result<ExtensionInstallation, ExtensionInstallationError> {
    ExtensionInstallation::from_persisted_parts(ExtensionInstallationPersistedParts {
        installation_id: installation.installation_id().clone(),
        extension_id,
        activation_state,
        manifest_ref,
        credential_bindings: installation.credential_bindings().to_vec(),
        health: installation.health().clone(),
        updated_at: installation.updated_at(),
        owner: installation.owner().clone(),
    })
}

fn persisted_manifest_id(raw_toml: &str) -> Result<String, ExtensionInstallationError> {
    let document = parse_persisted_manifest_toml(raw_toml)?;
    document
        .as_table()
        .and_then(|root| root.get("id"))
        .and_then(toml::Value::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| invalid_installation_error("persisted extension manifest is missing id"))
}

fn parse_persisted_manifest_toml(
    raw_toml: &str,
) -> Result<toml::Value, ExtensionInstallationError> {
    if raw_toml.len() > MAX_MANIFEST_BYTES {
        return Err(ManifestV2Error::ManifestTooLarge {
            bytes: raw_toml.len(),
            max: MAX_MANIFEST_BYTES,
        }
        .into());
    }
    toml::from_str(raw_toml).map_err(|error| {
        ManifestV2Error::Parse {
            reason: error.to_string(),
        }
        .into()
    })
}

#[cfg(feature = "slack-v2-host-beta")]
fn bundled_slack_wire_manifest() -> Result<WireManifestRecord, ExtensionInstallationError> {
    use super::available_extensions::{
        AvailableExtensionCatalog, SLACK_EXTENSION_ID, slack_manifest_digest,
    };

    let catalog = AvailableExtensionCatalog::from_first_party_assets_with_nearai_mcp_config(None)
        .map_err(invalid_installation_error)?;
    let package = catalog
        .search(SLACK_EXTENSION_ID)
        .find(|package| package.package_ref.id.as_str() == SLACK_EXTENSION_ID)
        .ok_or_else(|| invalid_installation_error("unified Slack manifest is unavailable"))?;
    Ok(WireManifestRecord {
        raw_toml: package.manifest_toml.clone(),
        source: WireManifestSource::from_manifest_source(package.source),
        manifest_hash: Some(
            ManifestHash::new(slack_manifest_digest()).map_err(invalid_installation_error)?,
        ),
        removal_cleanup_requirements: package.cleanup_requirements.clone(),
    })
}

#[cfg(not(feature = "slack-v2-host-beta"))]
fn bundled_slack_wire_manifest() -> Result<WireManifestRecord, ExtensionInstallationError> {
    Err(invalid_installation_error(
        "unified Slack manifest is unavailable in this build",
    ))
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
        ExtensionManifestRecord::from_toml(
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
