use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ironclaw_host_api::{ExtensionId, HostPortCatalog, SecretHandle};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use thiserror::Error;
use tokio::sync::RwLock;

use crate::{ExtensionManifestV2, HostApiContractRegistry, ManifestSource, ManifestV2Error};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct ManifestHash(String);

impl ManifestHash {
    pub fn new(value: impl Into<String>) -> Result<Self, ExtensionInstallationError> {
        let value = value.into();
        validate_nonempty_noncontrol("manifest_hash", &value)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for ManifestHash {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

/// Product-agnostic extension manifest record.
///
/// Domain crates can project their own host-api sections from `raw_toml` and
/// `manifest`; installation state itself stays in `ironclaw_extensions`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionManifestRecord {
    raw_toml: String,
    manifest: ExtensionManifestV2,
    manifest_hash: Option<ManifestHash>,
}

impl ExtensionManifestRecord {
    pub fn from_toml(
        raw_toml: impl Into<String>,
        source: ManifestSource,
        host_port_catalog: &HostPortCatalog,
        manifest_hash: Option<ManifestHash>,
    ) -> Result<Self, ExtensionInstallationError> {
        let contracts = HostApiContractRegistry::new();
        Self::from_toml_with_contracts(
            raw_toml,
            source,
            host_port_catalog,
            manifest_hash,
            &contracts,
        )
    }

    pub fn from_toml_with_contracts(
        raw_toml: impl Into<String>,
        source: ManifestSource,
        host_port_catalog: &HostPortCatalog,
        manifest_hash: Option<ManifestHash>,
        contracts: &HostApiContractRegistry,
    ) -> Result<Self, ExtensionInstallationError> {
        let raw_toml = raw_toml.into();
        let manifest = ExtensionManifestV2::parse_with_optional_host_api_contracts(
            &raw_toml,
            source,
            host_port_catalog,
            contracts,
        )?;
        Ok(Self {
            raw_toml,
            manifest,
            manifest_hash,
        })
    }

    pub fn manifest(&self) -> &ExtensionManifestV2 {
        &self.manifest
    }

    pub fn raw_toml(&self) -> &str {
        &self.raw_toml
    }

    pub fn extension_id(&self) -> &ExtensionId {
        &self.manifest.id
    }

    pub fn manifest_hash(&self) -> Option<&ManifestHash> {
        self.manifest_hash.as_ref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct ExtensionInstallationId(String);

impl ExtensionInstallationId {
    pub fn new(value: impl Into<String>) -> Result<Self, ExtensionInstallationError> {
        let value = value.into();
        validate_nonempty_noncontrol("installation_id", &value)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ExtensionInstallationId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for ExtensionInstallationId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct ExtensionCredentialHandle(String);

impl ExtensionCredentialHandle {
    pub fn new(value: impl Into<String>) -> Result<Self, ExtensionInstallationError> {
        let value = value.into();
        validate_nonempty_noncontrol("credential_handle", &value)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ExtensionCredentialHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ExtensionCredentialHandle {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtensionActivationState {
    Installed,
    Disabled,
    Enabled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtensionCredentialBinding {
    credential_handle: ExtensionCredentialHandle,
    secret_handle: SecretHandle,
}

impl ExtensionCredentialBinding {
    pub fn new(credential_handle: ExtensionCredentialHandle, secret_handle: SecretHandle) -> Self {
        Self {
            credential_handle,
            secret_handle,
        }
    }

    pub fn credential_handle(&self) -> &ExtensionCredentialHandle {
        &self.credential_handle
    }

    pub fn secret_handle(&self) -> &SecretHandle {
        &self.secret_handle
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtensionManifestRef {
    extension_id: ExtensionId,
    manifest_hash: Option<ManifestHash>,
}

impl ExtensionManifestRef {
    pub fn new(extension_id: ExtensionId, manifest_hash: Option<ManifestHash>) -> Self {
        Self {
            extension_id,
            manifest_hash,
        }
    }

    pub fn extension_id(&self) -> &ExtensionId {
        &self.extension_id
    }

    pub fn manifest_hash(&self) -> Option<&ManifestHash> {
        self.manifest_hash.as_ref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtensionHealthSnapshot {
    status: ExtensionHealthStatus,
    message: Option<ExtensionHealthMessage>,
    checked_at: DateTime<Utc>,
}

const REDACTED_PLACEHOLDER: &str = "<redacted>";

#[derive(Clone, PartialEq, Eq)]
pub struct ExtensionHealthMessage(String);

impl ExtensionHealthMessage {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn placeholder() -> &'static str {
        REDACTED_PLACEHOLDER
    }
}

impl fmt::Debug for ExtensionHealthMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(REDACTED_PLACEHOLDER)
    }
}

impl fmt::Display for ExtensionHealthMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(REDACTED_PLACEHOLDER)
    }
}

impl Serialize for ExtensionHealthMessage {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(REDACTED_PLACEHOLDER)
    }
}

impl<'de> Deserialize<'de> for ExtensionHealthMessage {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        String::deserialize(deserializer).map(|_| Self(REDACTED_PLACEHOLDER.to_string()))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtensionHealthStatus {
    Healthy,
    Degraded,
    Unhealthy,
}

impl ExtensionHealthSnapshot {
    pub fn healthy() -> Self {
        Self {
            status: ExtensionHealthStatus::Healthy,
            message: None,
            checked_at: Utc::now(),
        }
    }

    pub fn new(
        status: ExtensionHealthStatus,
        message: Option<ExtensionHealthMessage>,
        checked_at: DateTime<Utc>,
    ) -> Self {
        Self {
            status,
            message,
            checked_at,
        }
    }

    pub fn status(&self) -> ExtensionHealthStatus {
        self.status
    }

    pub fn message(&self) -> Option<&ExtensionHealthMessage> {
        self.message.as_ref()
    }

    pub fn checked_at(&self) -> DateTime<Utc> {
        self.checked_at
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExtensionInstallation {
    installation_id: ExtensionInstallationId,
    extension_id: ExtensionId,
    activation_state: ExtensionActivationState,
    manifest_ref: ExtensionManifestRef,
    credential_bindings: Vec<ExtensionCredentialBinding>,
    health: ExtensionHealthSnapshot,
    updated_at: DateTime<Utc>,
}

impl ExtensionInstallation {
    pub fn new(
        installation_id: ExtensionInstallationId,
        extension_id: ExtensionId,
        activation_state: ExtensionActivationState,
        manifest_ref: ExtensionManifestRef,
        credential_bindings: Vec<ExtensionCredentialBinding>,
        updated_at: DateTime<Utc>,
    ) -> Result<Self, ExtensionInstallationError> {
        if manifest_ref.extension_id() != &extension_id {
            return Err(ExtensionInstallationError::ManifestExtensionMismatch {
                extension_id,
                manifest_extension_id: manifest_ref.extension_id().clone(),
            });
        }
        validate_bindings_unique(&credential_bindings)?;
        Ok(Self {
            installation_id,
            extension_id,
            activation_state,
            manifest_ref,
            credential_bindings,
            health: ExtensionHealthSnapshot::new(ExtensionHealthStatus::Healthy, None, updated_at),
            updated_at,
        })
    }

    pub fn installation_id(&self) -> &ExtensionInstallationId {
        &self.installation_id
    }

    pub fn extension_id(&self) -> &ExtensionId {
        &self.extension_id
    }

    pub fn activation_state(&self) -> ExtensionActivationState {
        self.activation_state
    }

    pub fn manifest_ref(&self) -> &ExtensionManifestRef {
        &self.manifest_ref
    }

    pub fn credential_bindings(&self) -> &[ExtensionCredentialBinding] {
        &self.credential_bindings
    }

    pub fn health(&self) -> &ExtensionHealthSnapshot {
        &self.health
    }

    pub fn updated_at(&self) -> DateTime<Utc> {
        self.updated_at
    }

    fn set_activation_state(&mut self, state: ExtensionActivationState) {
        self.activation_state = state;
        self.updated_at = Utc::now();
    }

    fn set_health(&mut self, health: ExtensionHealthSnapshot) {
        self.health = health;
        self.updated_at = Utc::now();
    }
}

impl<'de> Deserialize<'de> for ExtensionInstallation {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            installation_id: ExtensionInstallationId,
            extension_id: ExtensionId,
            activation_state: ExtensionActivationState,
            manifest_ref: ExtensionManifestRef,
            credential_bindings: Vec<ExtensionCredentialBinding>,
            health: ExtensionHealthSnapshot,
            updated_at: DateTime<Utc>,
        }
        let wire = Wire::deserialize(deserializer)?;
        if wire.manifest_ref.extension_id() != &wire.extension_id {
            return Err(serde::de::Error::custom(
                ExtensionInstallationError::ManifestExtensionMismatch {
                    extension_id: wire.extension_id,
                    manifest_extension_id: wire.manifest_ref.extension_id().clone(),
                },
            ));
        }
        validate_bindings_unique(&wire.credential_bindings).map_err(serde::de::Error::custom)?;
        Ok(Self {
            installation_id: wire.installation_id,
            extension_id: wire.extension_id,
            activation_state: wire.activation_state,
            manifest_ref: wire.manifest_ref,
            credential_bindings: wire.credential_bindings,
            health: wire.health,
            updated_at: wire.updated_at,
        })
    }
}

/// Generic extension installation state store.
///
/// Implementations own product-agnostic manifest records, installation
/// activation state, opaque credential bindings, health snapshots, and
/// manifest-hash consistency. Domain crates validate domain-specific binding
/// semantics when projecting their host-api sections from these records.
/// `list_enabled_installations` returns enabled installations in
/// newest-updated order with a deterministic installation-id tie-breaker.
#[async_trait]
pub trait ExtensionInstallationStore: Send + Sync {
    async fn list_manifests(
        &self,
    ) -> Result<Vec<ExtensionManifestRecord>, ExtensionInstallationError>;

    async fn get_manifest(
        &self,
        extension_id: &ExtensionId,
    ) -> Result<Option<ExtensionManifestRecord>, ExtensionInstallationError>;

    async fn upsert_manifest(
        &self,
        manifest: ExtensionManifestRecord,
    ) -> Result<(), ExtensionInstallationError>;

    async fn list_installations(
        &self,
    ) -> Result<Vec<ExtensionInstallation>, ExtensionInstallationError>;

    async fn list_enabled_installations(
        &self,
    ) -> Result<Vec<ExtensionInstallation>, ExtensionInstallationError>;

    async fn get_installation(
        &self,
        installation_id: &ExtensionInstallationId,
    ) -> Result<Option<ExtensionInstallation>, ExtensionInstallationError>;

    async fn upsert_installation(
        &self,
        installation: ExtensionInstallation,
    ) -> Result<(), ExtensionInstallationError>;

    async fn set_activation_state(
        &self,
        installation_id: &ExtensionInstallationId,
        state: ExtensionActivationState,
    ) -> Result<(), ExtensionInstallationError>;

    async fn delete_installation(
        &self,
        installation_id: &ExtensionInstallationId,
    ) -> Result<(), ExtensionInstallationError>;

    async fn delete_manifest(
        &self,
        extension_id: &ExtensionId,
    ) -> Result<(), ExtensionInstallationError>;

    async fn update_health(
        &self,
        installation_id: &ExtensionInstallationId,
        health: ExtensionHealthSnapshot,
    ) -> Result<(), ExtensionInstallationError>;
}

#[async_trait]
impl<T> ExtensionInstallationStore for Arc<T>
where
    T: ExtensionInstallationStore + ?Sized,
{
    async fn list_manifests(
        &self,
    ) -> Result<Vec<ExtensionManifestRecord>, ExtensionInstallationError> {
        (**self).list_manifests().await
    }

    async fn get_manifest(
        &self,
        extension_id: &ExtensionId,
    ) -> Result<Option<ExtensionManifestRecord>, ExtensionInstallationError> {
        (**self).get_manifest(extension_id).await
    }

    async fn upsert_manifest(
        &self,
        manifest: ExtensionManifestRecord,
    ) -> Result<(), ExtensionInstallationError> {
        (**self).upsert_manifest(manifest).await
    }

    async fn list_installations(
        &self,
    ) -> Result<Vec<ExtensionInstallation>, ExtensionInstallationError> {
        (**self).list_installations().await
    }

    async fn list_enabled_installations(
        &self,
    ) -> Result<Vec<ExtensionInstallation>, ExtensionInstallationError> {
        (**self).list_enabled_installations().await
    }

    async fn get_installation(
        &self,
        installation_id: &ExtensionInstallationId,
    ) -> Result<Option<ExtensionInstallation>, ExtensionInstallationError> {
        (**self).get_installation(installation_id).await
    }

    async fn upsert_installation(
        &self,
        installation: ExtensionInstallation,
    ) -> Result<(), ExtensionInstallationError> {
        (**self).upsert_installation(installation).await
    }

    async fn set_activation_state(
        &self,
        installation_id: &ExtensionInstallationId,
        state: ExtensionActivationState,
    ) -> Result<(), ExtensionInstallationError> {
        (**self).set_activation_state(installation_id, state).await
    }

    async fn delete_installation(
        &self,
        installation_id: &ExtensionInstallationId,
    ) -> Result<(), ExtensionInstallationError> {
        (**self).delete_installation(installation_id).await
    }

    async fn delete_manifest(
        &self,
        extension_id: &ExtensionId,
    ) -> Result<(), ExtensionInstallationError> {
        (**self).delete_manifest(extension_id).await
    }

    async fn update_health(
        &self,
        installation_id: &ExtensionInstallationId,
        health: ExtensionHealthSnapshot,
    ) -> Result<(), ExtensionInstallationError> {
        (**self).update_health(installation_id, health).await
    }
}

#[derive(Debug, Default, Clone)]
pub struct InMemoryExtensionInstallationStore {
    inner: Arc<RwLock<InMemoryState>>,
}

#[derive(Debug, Default)]
struct InMemoryState {
    manifests: HashMap<ExtensionId, ExtensionManifestRecord>,
    installations: HashMap<ExtensionInstallationId, ExtensionInstallation>,
}

#[async_trait]
impl ExtensionInstallationStore for InMemoryExtensionInstallationStore {
    async fn list_manifests(
        &self,
    ) -> Result<Vec<ExtensionManifestRecord>, ExtensionInstallationError> {
        let inner = self.inner.read().await;
        let mut manifests: Vec<_> = inner.manifests.values().cloned().collect();
        manifests.sort_by(|a, b| a.extension_id().cmp(b.extension_id()));
        Ok(manifests)
    }

    async fn get_manifest(
        &self,
        extension_id: &ExtensionId,
    ) -> Result<Option<ExtensionManifestRecord>, ExtensionInstallationError> {
        Ok(self.inner.read().await.manifests.get(extension_id).cloned())
    }

    async fn upsert_manifest(
        &self,
        manifest: ExtensionManifestRecord,
    ) -> Result<(), ExtensionInstallationError> {
        let mut inner = self.inner.write().await;
        // The in-memory store intentionally scans existing installations to
        // preserve the manifest hash invariant. Durable stores should use a
        // targeted read path or secondary index before handling large catalogs.
        for installation in inner.installations.values() {
            if installation.extension_id() == manifest.extension_id() {
                validate_installation_against_one_manifest(&manifest, installation)?;
            }
        }
        inner
            .manifests
            .insert(manifest.extension_id().clone(), manifest);
        Ok(())
    }

    async fn list_installations(
        &self,
    ) -> Result<Vec<ExtensionInstallation>, ExtensionInstallationError> {
        let inner = self.inner.read().await;
        let mut installations: Vec<_> = inner.installations.values().cloned().collect();
        installations.sort_by(|a, b| a.installation_id().cmp(b.installation_id()));
        Ok(installations)
    }

    async fn list_enabled_installations(
        &self,
    ) -> Result<Vec<ExtensionInstallation>, ExtensionInstallationError> {
        let inner = self.inner.read().await;
        let mut installations: Vec<_> = inner
            .installations
            .values()
            .filter(|i| i.activation_state() == ExtensionActivationState::Enabled)
            .cloned()
            .collect();
        installations.sort_by(|a, b| {
            b.updated_at()
                .cmp(&a.updated_at())
                .then_with(|| a.installation_id().cmp(b.installation_id()))
        });
        Ok(installations)
    }

    async fn get_installation(
        &self,
        installation_id: &ExtensionInstallationId,
    ) -> Result<Option<ExtensionInstallation>, ExtensionInstallationError> {
        Ok(self
            .inner
            .read()
            .await
            .installations
            .get(installation_id)
            .cloned())
    }

    async fn upsert_installation(
        &self,
        installation: ExtensionInstallation,
    ) -> Result<(), ExtensionInstallationError> {
        let mut inner = self.inner.write().await;
        validate_installation_against_manifest(&inner.manifests, &installation)?;
        inner
            .installations
            .insert(installation.installation_id().clone(), installation);
        Ok(())
    }

    async fn set_activation_state(
        &self,
        installation_id: &ExtensionInstallationId,
        state: ExtensionActivationState,
    ) -> Result<(), ExtensionInstallationError> {
        let mut inner = self.inner.write().await;
        let InMemoryState {
            manifests,
            installations,
        } = &mut *inner;
        let installation = installations.get_mut(installation_id).ok_or_else(|| {
            ExtensionInstallationError::InstallationNotFound {
                installation_id: installation_id.clone(),
            }
        })?;
        if installation.activation_state() == state {
            return Ok(());
        }
        if state == ExtensionActivationState::Enabled {
            validate_installation_against_manifest(manifests, installation)?;
        }
        installation.set_activation_state(state);
        Ok(())
    }

    async fn delete_installation(
        &self,
        installation_id: &ExtensionInstallationId,
    ) -> Result<(), ExtensionInstallationError> {
        self.inner
            .write()
            .await
            .installations
            .remove(installation_id)
            .map(|_| ())
            .ok_or_else(|| ExtensionInstallationError::InstallationNotFound {
                installation_id: installation_id.clone(),
            })
    }

    async fn delete_manifest(
        &self,
        extension_id: &ExtensionId,
    ) -> Result<(), ExtensionInstallationError> {
        if self
            .inner
            .read()
            .await
            .installations
            .values()
            .any(|installation| installation.extension_id() == extension_id)
        {
            return Err(ExtensionInstallationError::InvalidInstallation {
                reason: format!("extension {extension_id} still has installations"),
            });
        }
        let mut inner = self.inner.write().await;
        if inner
            .installations
            .values()
            .any(|installation| installation.extension_id() == extension_id)
        {
            return Err(ExtensionInstallationError::InvalidInstallation {
                reason: format!("extension {extension_id} still has installations"),
            });
        }
        inner
            .manifests
            .remove(extension_id)
            .map(|_| ())
            .ok_or_else(|| ExtensionInstallationError::ManifestNotFound {
                extension_id: extension_id.clone(),
            })
    }

    async fn update_health(
        &self,
        installation_id: &ExtensionInstallationId,
        health: ExtensionHealthSnapshot,
    ) -> Result<(), ExtensionInstallationError> {
        self.inner
            .write()
            .await
            .installations
            .get_mut(installation_id)
            .ok_or_else(|| ExtensionInstallationError::InstallationNotFound {
                installation_id: installation_id.clone(),
            })?
            .set_health(health);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use ironclaw_host_api::{ExtensionId, HostPortCatalog};

    use super::*;
    use crate::ManifestSource;

    #[tokio::test]
    async fn delete_manifest_rejects_active_installations() {
        let store = InMemoryExtensionInstallationStore::default();
        let manifest = manifest_record("fixture", Some("hash-1"));
        let extension_id = manifest.extension_id().clone();
        store
            .upsert_manifest(manifest)
            .await
            .expect("upsert manifest");
        store
            .upsert_installation(installation("fixture", Some("hash-1")))
            .await
            .expect("upsert installation");

        let error = store
            .delete_manifest(&extension_id)
            .await
            .expect_err("active installation blocks manifest delete");

        assert!(matches!(
            error,
            ExtensionInstallationError::InvalidInstallation { .. }
        ));
        assert!(store.get_manifest(&extension_id).await.unwrap().is_some());
    }

    fn manifest_record(extension_id: &str, hash: Option<&str>) -> ExtensionManifestRecord {
        ExtensionManifestRecord::from_toml(
            manifest_toml(extension_id),
            ManifestSource::HostBundled,
            &HostPortCatalog::empty(),
            hash.map(|value| ManifestHash::new(value).expect("hash")),
        )
        .expect("manifest record")
    }

    fn installation(extension_id: &str, hash: Option<&str>) -> ExtensionInstallation {
        let extension_id = ExtensionId::new(extension_id.to_string()).expect("extension id");
        ExtensionInstallation::new(
            ExtensionInstallationId::new(extension_id.as_str().to_string())
                .expect("installation id"),
            extension_id.clone(),
            ExtensionActivationState::Installed,
            ExtensionManifestRef::new(
                extension_id,
                hash.map(|value| ManifestHash::new(value).expect("hash")),
            ),
            Vec::new(),
            Utc::now(),
        )
        .expect("installation")
    }

    fn manifest_toml(extension_id: &str) -> String {
        format!(
            r#"
schema_version = "reborn.extension_manifest.v2"
id = "{extension_id}"
name = "{extension_id}"
version = "0.1.0"
description = "test extension"
trust = "third_party"

[runtime]
kind = "wasm"
module = "wasm/{extension_id}.wasm"

[[capabilities]]
id = "{extension_id}.read"
description = "read"
effects = ["network"]
default_permission = "ask"
visibility = "model"
input_schema_ref = "schemas/read.input.json"
output_schema_ref = "schemas/read.output.json"
"#
        )
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ExtensionInstallationError {
    #[error(transparent)]
    Manifest(#[from] ManifestV2Error),
    #[error("invalid {field}: {reason}")]
    InvalidValue { field: &'static str, reason: String },
    #[error("installation references unknown extension manifest {extension_id}")]
    UnknownManifest { extension_id: ExtensionId },
    #[error(
        "installation extension {extension_id} does not match manifest extension {manifest_extension_id}"
    )]
    ManifestExtensionMismatch {
        extension_id: ExtensionId,
        manifest_extension_id: ExtensionId,
    },
    #[error(
        "installation manifest hash does not match registered manifest hash for {extension_id}"
    )]
    ManifestHashMismatch { extension_id: ExtensionId },
    #[error("installation {installation_id} was not found")]
    InstallationNotFound {
        installation_id: ExtensionInstallationId,
    },
    #[error("extension manifest {extension_id} was not found")]
    ManifestNotFound { extension_id: ExtensionId },
    #[error("invalid installation: {reason}")]
    InvalidInstallation { reason: String },
    #[error("duplicate credential binding {handle}")]
    DuplicateCredentialBinding { handle: ExtensionCredentialHandle },
}

fn validate_installation_against_manifest(
    manifests: &HashMap<ExtensionId, ExtensionManifestRecord>,
    installation: &ExtensionInstallation,
) -> Result<(), ExtensionInstallationError> {
    let manifest = manifests.get(installation.extension_id()).ok_or_else(|| {
        ExtensionInstallationError::UnknownManifest {
            extension_id: installation.extension_id().clone(),
        }
    })?;
    validate_installation_against_one_manifest(manifest, installation)
}

fn validate_installation_against_one_manifest(
    manifest: &ExtensionManifestRecord,
    installation: &ExtensionInstallation,
) -> Result<(), ExtensionInstallationError> {
    if manifest.extension_id() != installation.manifest_ref().extension_id() {
        return Err(ExtensionInstallationError::ManifestExtensionMismatch {
            extension_id: installation.extension_id().clone(),
            manifest_extension_id: installation.manifest_ref().extension_id().clone(),
        });
    }
    match (
        manifest.manifest_hash(),
        installation.manifest_ref().manifest_hash(),
    ) {
        (Some(registered), Some(referenced)) if registered != referenced => {
            return Err(ExtensionInstallationError::ManifestHashMismatch {
                extension_id: installation.extension_id().clone(),
            });
        }
        (Some(_), None) | (None, Some(_)) => {
            return Err(ExtensionInstallationError::ManifestHashMismatch {
                extension_id: installation.extension_id().clone(),
            });
        }
        _ => {}
    }
    Ok(())
}

fn validate_bindings_unique(
    credential_bindings: &[ExtensionCredentialBinding],
) -> Result<(), ExtensionInstallationError> {
    let mut seen = std::collections::BTreeSet::new();
    for binding in credential_bindings {
        if !seen.insert(binding.credential_handle.clone()) {
            return Err(ExtensionInstallationError::DuplicateCredentialBinding {
                handle: binding.credential_handle.clone(),
            });
        }
    }
    Ok(())
}

fn validate_nonempty_noncontrol(
    field: &'static str,
    value: &str,
) -> Result<(), ExtensionInstallationError> {
    if value.is_empty() {
        return Err(ExtensionInstallationError::InvalidValue {
            field,
            reason: "must not be empty".to_string(),
        });
    }
    if value.chars().any(|c| c == '\0' || c.is_control()) {
        return Err(ExtensionInstallationError::InvalidValue {
            field,
            reason: "must not contain control characters".to_string(),
        });
    }
    Ok(())
}
