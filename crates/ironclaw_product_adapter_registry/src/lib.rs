//! Generic extension installation registry with ProductAdapter projection.
//!
//! There is one store, one installation type, one manifest type. The
//! ProductAdapter-specific path is a projection on top:
//!
//! ```text
//! ExtensionInstallationStore (generic, async, persistence-backed)
//!   manifests    → ExtensionManifestRecord   (any ExtensionManifestV2)
//!   installations → ExtensionInstallation    (any extension, keyed by ExtensionId)
//!
//! list_enabled_product_adapter_entries(store)
//!   → filter enabled installations whose manifest carries ironclaw.product_adapter/v1
//!   → project ProductAdapterHostApiSection from that section
//!   → return Vec<ProductAdapterRuntimeEntry>
//! ```

#![forbid(unsafe_code)]

use std::collections::{BTreeSet, HashMap};
use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ironclaw_extensions::{
    ExtensionManifestV2, HostApiContractRegistry, HostApiId, HostApiManifestContract,
    HostApiMultiplicity, HostApiRefV2, ManifestSectionPath, ManifestSource, ManifestV2Error,
};
use ironclaw_host_api::{ExtensionId, HostPortCatalog, SecretHandle};
use ironclaw_product_adapters::{
    AuthRequirement, DeclaredEgressTarget, EgressCredentialHandle, ProductAdapterCapabilities,
    ProductAdapterId, ProductCapabilityFlag, ProductSurfaceKind, RedactedString,
};
use serde::{Deserialize, Deserializer, Serialize};
use thiserror::Error;
use tokio::sync::RwLock;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

pub const PRODUCT_ADAPTER_HOST_API_ID: &str = "ironclaw.product_adapter/v1";
pub const PRODUCT_ADAPTER_SECTION_PREFIX: &str = "product_adapter";

// ---------------------------------------------------------------------------
// Shared primitives
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct ManifestHash(String);

impl ManifestHash {
    pub fn new(value: impl Into<String>) -> Result<Self, RegistryError> {
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

// ---------------------------------------------------------------------------
// ExtensionManifestRecord — single manifest type for any extension
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionManifestRecord {
    raw_toml: String,
    manifest: ExtensionManifestV2,
    product_adapters: Vec<ProductAdapterHostApiSection>,
    manifest_hash: Option<ManifestHash>,
}

impl ExtensionManifestRecord {
    /// Parse a single Extension Manifest v2 TOML document. ProductAdapter
    /// host-api sections are projected automatically when present.
    pub fn from_toml(
        raw_toml: impl Into<String>,
        source: ManifestSource,
        host_port_catalog: &HostPortCatalog,
        manifest_hash: Option<ManifestHash>,
    ) -> Result<Self, RegistryError> {
        let contract = Arc::new(ProductAdapterHostApiContract::new()?);
        let mut contracts = HostApiContractRegistry::new();
        contracts.register(contract)?;
        Self::from_toml_with_contracts(
            raw_toml,
            source,
            host_port_catalog,
            manifest_hash,
            &contracts,
        )
    }

    /// Parse with a caller-supplied contract registry (for additional host API types).
    pub fn from_toml_with_contracts(
        raw_toml: impl Into<String>,
        source: ManifestSource,
        host_port_catalog: &HostPortCatalog,
        manifest_hash: Option<ManifestHash>,
        contracts: &HostApiContractRegistry,
    ) -> Result<Self, RegistryError> {
        let raw_toml = raw_toml.into();
        let manifest = ExtensionManifestV2::parse_with_host_api_contracts(
            &raw_toml,
            source,
            host_port_catalog,
            contracts,
        )?;
        let product_adapters = project_product_adapter_sections(&raw_toml, &manifest)?;
        Ok(Self {
            raw_toml,
            manifest,
            product_adapters,
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

    pub fn product_adapters(&self) -> &[ProductAdapterHostApiSection] {
        &self.product_adapters
    }

    pub fn manifest_hash(&self) -> Option<&ManifestHash> {
        self.manifest_hash.as_ref()
    }
}

// ---------------------------------------------------------------------------
// Generic extension installation state
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct ExtensionInstallationId(String);

impl ExtensionInstallationId {
    pub fn new(value: impl Into<String>) -> Result<Self, RegistryError> {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtensionActivationState {
    Installed,
    Disabled,
    Enabled,
}

/// Opaque credential binding: maps a manifest-declared handle name to a
/// host secret handle. Never stores raw secret material.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtensionCredentialBinding {
    credential_handle: EgressCredentialHandle,
    secret_handle: SecretHandle,
}

impl ExtensionCredentialBinding {
    pub fn new(credential_handle: EgressCredentialHandle, secret_handle: SecretHandle) -> Self {
        Self {
            credential_handle,
            secret_handle,
        }
    }

    pub fn credential_handle(&self) -> &EgressCredentialHandle {
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
    message: Option<RedactedString>,
    checked_at: DateTime<Utc>,
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
        message: Option<RedactedString>,
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

    pub fn message(&self) -> Option<&RedactedString> {
        self.message.as_ref()
    }

    pub fn checked_at(&self) -> DateTime<Utc> {
        self.checked_at
    }
}

/// One installed extension with activation state and opaque credential bindings.
/// Works for any extension kind — ProductAdapter filtering is a projection.
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
    ) -> Result<Self, RegistryError> {
        if manifest_ref.extension_id() != &extension_id {
            return Err(RegistryError::ManifestExtensionMismatch {
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
            health: ExtensionHealthSnapshot::healthy(),
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
                RegistryError::ManifestExtensionMismatch {
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

// ---------------------------------------------------------------------------
// Single generic store trait
// ---------------------------------------------------------------------------

#[async_trait]
pub trait ExtensionInstallationStore: Send + Sync {
    async fn list_manifests(&self) -> Result<Vec<ExtensionManifestRecord>, RegistryError>;

    async fn get_manifest(
        &self,
        extension_id: &ExtensionId,
    ) -> Result<Option<ExtensionManifestRecord>, RegistryError>;

    async fn upsert_manifest(&self, manifest: ExtensionManifestRecord)
    -> Result<(), RegistryError>;

    async fn list_installations(&self) -> Result<Vec<ExtensionInstallation>, RegistryError>;

    async fn list_enabled_installations(&self)
    -> Result<Vec<ExtensionInstallation>, RegistryError>;

    async fn get_installation(
        &self,
        installation_id: &ExtensionInstallationId,
    ) -> Result<Option<ExtensionInstallation>, RegistryError>;

    async fn upsert_installation(
        &self,
        installation: ExtensionInstallation,
    ) -> Result<(), RegistryError>;

    async fn set_activation_state(
        &self,
        installation_id: &ExtensionInstallationId,
        state: ExtensionActivationState,
    ) -> Result<(), RegistryError>;

    /// Replace the stored health snapshot without enforcing status/message
    /// consistency; callers may report status codes before a diagnostic exists.
    async fn update_health(
        &self,
        installation_id: &ExtensionInstallationId,
        health: ExtensionHealthSnapshot,
    ) -> Result<(), RegistryError>;
}

#[async_trait]
impl<T> ExtensionInstallationStore for Arc<T>
where
    T: ExtensionInstallationStore + ?Sized,
{
    async fn list_manifests(&self) -> Result<Vec<ExtensionManifestRecord>, RegistryError> {
        (**self).list_manifests().await
    }

    async fn get_manifest(
        &self,
        extension_id: &ExtensionId,
    ) -> Result<Option<ExtensionManifestRecord>, RegistryError> {
        (**self).get_manifest(extension_id).await
    }

    async fn upsert_manifest(
        &self,
        manifest: ExtensionManifestRecord,
    ) -> Result<(), RegistryError> {
        (**self).upsert_manifest(manifest).await
    }

    async fn list_installations(&self) -> Result<Vec<ExtensionInstallation>, RegistryError> {
        (**self).list_installations().await
    }

    async fn list_enabled_installations(
        &self,
    ) -> Result<Vec<ExtensionInstallation>, RegistryError> {
        (**self).list_enabled_installations().await
    }

    async fn get_installation(
        &self,
        installation_id: &ExtensionInstallationId,
    ) -> Result<Option<ExtensionInstallation>, RegistryError> {
        (**self).get_installation(installation_id).await
    }

    async fn upsert_installation(
        &self,
        installation: ExtensionInstallation,
    ) -> Result<(), RegistryError> {
        (**self).upsert_installation(installation).await
    }

    async fn set_activation_state(
        &self,
        installation_id: &ExtensionInstallationId,
        state: ExtensionActivationState,
    ) -> Result<(), RegistryError> {
        (**self).set_activation_state(installation_id, state).await
    }

    async fn update_health(
        &self,
        installation_id: &ExtensionInstallationId,
        health: ExtensionHealthSnapshot,
    ) -> Result<(), RegistryError> {
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
    async fn list_manifests(&self) -> Result<Vec<ExtensionManifestRecord>, RegistryError> {
        let inner = self.inner.read().await;
        let mut manifests: Vec<_> = inner.manifests.values().cloned().collect();
        manifests.sort_by(|a, b| a.extension_id().cmp(b.extension_id()));
        Ok(manifests)
    }

    async fn get_manifest(
        &self,
        extension_id: &ExtensionId,
    ) -> Result<Option<ExtensionManifestRecord>, RegistryError> {
        Ok(self.inner.read().await.manifests.get(extension_id).cloned())
    }

    async fn upsert_manifest(
        &self,
        manifest: ExtensionManifestRecord,
    ) -> Result<(), RegistryError> {
        let mut inner = self.inner.write().await;
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

    async fn list_installations(&self) -> Result<Vec<ExtensionInstallation>, RegistryError> {
        let inner = self.inner.read().await;
        let mut installations: Vec<_> = inner.installations.values().cloned().collect();
        installations.sort_by(|a, b| a.installation_id().cmp(b.installation_id()));
        Ok(installations)
    }

    async fn list_enabled_installations(
        &self,
    ) -> Result<Vec<ExtensionInstallation>, RegistryError> {
        Ok(self
            .list_installations()
            .await?
            .into_iter()
            .filter(|i| i.activation_state() == ExtensionActivationState::Enabled)
            .collect())
    }

    async fn get_installation(
        &self,
        installation_id: &ExtensionInstallationId,
    ) -> Result<Option<ExtensionInstallation>, RegistryError> {
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
    ) -> Result<(), RegistryError> {
        validate_bindings_unique(installation.credential_bindings())?;
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
    ) -> Result<(), RegistryError> {
        let mut inner = self.inner.write().await;
        let InMemoryState {
            manifests,
            installations,
        } = &mut *inner;
        let installation = installations.get_mut(installation_id).ok_or_else(|| {
            RegistryError::InstallationNotFound {
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

    async fn update_health(
        &self,
        installation_id: &ExtensionInstallationId,
        health: ExtensionHealthSnapshot,
    ) -> Result<(), RegistryError> {
        self.inner
            .write()
            .await
            .installations
            .get_mut(installation_id)
            .ok_or_else(|| RegistryError::InstallationNotFound {
                installation_id: installation_id.clone(),
            })?
            .set_health(health);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// ProductAdapter projection
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductAdapterHostApiSection {
    adapter_id: ProductAdapterId,
    section: ManifestSectionPath,
    surface_kind: ProductSurfaceKind,
    capabilities: ProductAdapterCapabilities,
    auth_requirement: AuthRequirement,
    declared_egress: Vec<DeclaredEgressTarget>,
    required_credentials: Vec<EgressCredentialHandle>,
}

impl ProductAdapterHostApiSection {
    fn from_value(
        extension_id: &ExtensionId,
        section: ManifestSectionPath,
        value: toml::Value,
    ) -> Result<Self, RegistryError> {
        reject_inline_secret_material_value(section.as_str(), &value)?;
        let raw: RawProductAdapterSection =
            value.try_into().map_err(|error: toml::de::Error| {
                RegistryError::ManifestSectionParse {
                    section: section.clone(),
                    reason: error.to_string(),
                }
            })?;
        // Derive adapter_id from the extension id and section subsection name
        // so that multiple product-adapter sections within the same extension
        // are distinguishable downstream.
        let subsection = section
            .as_str()
            .strip_prefix(PRODUCT_ADAPTER_SECTION_PREFIX)
            .and_then(|rest| rest.strip_prefix('.'))
            .unwrap_or("default");
        let adapter_id_str = format!("{}/{}", extension_id.as_str(), subsection);
        let adapter_id = ProductAdapterId::new(&adapter_id_str).map_err(|error| {
            RegistryError::InvalidValue {
                field: "adapter_id",
                reason: error.to_string(),
            }
        })?;
        let auth_requirement = raw.auth.into_auth_requirement()?;
        let required_credentials = raw
            .required_credentials
            .into_iter()
            .map(|c| c.handle)
            .collect();
        let projected = Self {
            adapter_id,
            section,
            surface_kind: raw.surface_kind,
            capabilities: ProductAdapterCapabilities::new(raw.capabilities.flags),
            auth_requirement,
            declared_egress: raw.egress,
            required_credentials,
        };
        projected.validate()?;
        Ok(projected)
    }

    pub fn adapter_id(&self) -> &ProductAdapterId {
        &self.adapter_id
    }
    pub fn section(&self) -> &ManifestSectionPath {
        &self.section
    }
    pub fn surface_kind(&self) -> ProductSurfaceKind {
        self.surface_kind
    }
    pub fn capabilities(&self) -> &ProductAdapterCapabilities {
        &self.capabilities
    }
    pub fn auth_requirement(&self) -> &AuthRequirement {
        &self.auth_requirement
    }
    pub fn declared_egress(&self) -> &[DeclaredEgressTarget] {
        &self.declared_egress
    }
    pub fn required_credentials(&self) -> &[EgressCredentialHandle] {
        &self.required_credentials
    }

    fn validate(&self) -> Result<(), RegistryError> {
        validate_auth_requirement(&self.auth_requirement)?;
        let mut required = BTreeSet::new();
        for handle in &self.required_credentials {
            if !required.insert(handle.clone()) {
                return Err(RegistryError::DuplicateCredentialHandle {
                    handle: handle.clone(),
                });
            }
        }
        let mut pairs = BTreeSet::new();
        for target in &self.declared_egress {
            if let Some(handle) = target.credential_handle.as_ref()
                && !required.contains(handle)
            {
                return Err(RegistryError::UndeclaredEgressCredentialHandle {
                    handle: handle.clone(),
                });
            }
            if !pairs.insert((target.host.clone(), target.credential_handle.clone())) {
                return Err(RegistryError::DuplicateEgressTarget);
            }
        }
        Ok(())
    }
}

/// Enabled extension installation paired with its projected ProductAdapter section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductAdapterRuntimeEntry {
    installation: ExtensionInstallation,
    adapter: ProductAdapterHostApiSection,
}

impl ProductAdapterRuntimeEntry {
    fn new(installation: ExtensionInstallation, adapter: ProductAdapterHostApiSection) -> Self {
        Self {
            installation,
            adapter,
        }
    }

    pub fn installation(&self) -> &ExtensionInstallation {
        &self.installation
    }
    pub fn adapter(&self) -> &ProductAdapterHostApiSection {
        &self.adapter
    }
}

/// Project enabled ProductAdapter runtime entries from any `ExtensionInstallationStore`.
///
/// Filters to enabled installations whose manifest carries an
/// `ironclaw.product_adapter/v1` host-api section, then pairs each with its
/// projected ProductAdapter section. Enabled extensions without ProductAdapter
/// sections are intentionally ignored by this projection, not reported as
/// unknown manifests. Results are sorted by installation id.
pub async fn list_enabled_product_adapter_entries(
    store: &dyn ExtensionInstallationStore,
) -> Result<Vec<ProductAdapterRuntimeEntry>, RegistryError> {
    let manifests = store.list_manifests().await?;
    let manifest_map: HashMap<_, _> = manifests
        .iter()
        .map(|m| (m.extension_id().clone(), m))
        .collect();
    let mut entries = Vec::new();
    for installation in store.list_enabled_installations().await? {
        let manifest = manifest_map
            .get(installation.extension_id())
            .ok_or_else(|| RegistryError::UnknownManifest {
                extension_id: installation.extension_id().clone(),
            })?;
        validate_installation_against_one_manifest(manifest, &installation)?;
        let adapters = manifest.product_adapters();
        if adapters.is_empty() {
            continue;
        }
        for adapter in adapters {
            entries.push(ProductAdapterRuntimeEntry::new(
                installation.clone(),
                adapter.clone(),
            ));
        }
    }
    Ok(entries)
}

// ---------------------------------------------------------------------------
// ProductAdapter host-api contract validator
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct ProductAdapterHostApiContract {
    id: HostApiId,
}

impl ProductAdapterHostApiContract {
    pub fn new() -> Result<Self, RegistryError> {
        Ok(Self {
            id: HostApiId::new(PRODUCT_ADAPTER_HOST_API_ID)?,
        })
    }
}

impl HostApiManifestContract for ProductAdapterHostApiContract {
    fn id(&self) -> &HostApiId {
        &self.id
    }

    fn multiplicity(&self) -> HostApiMultiplicity {
        HostApiMultiplicity::Multiple
    }

    fn accepts_section_path(&self, section: &ManifestSectionPath) -> bool {
        section.as_str() == PRODUCT_ADAPTER_SECTION_PREFIX
            || section
                .as_str()
                .strip_prefix(PRODUCT_ADAPTER_SECTION_PREFIX)
                .is_some_and(|rest| rest.starts_with('.'))
    }

    fn validate_section(
        &self,
        host_api: &HostApiRefV2,
        section: &toml::Value,
    ) -> Result<(), String> {
        // The contract hook runs while the generic manifest parser is still
        // validating the host-api section envelope, before it exposes the real
        // extension id to contract implementations. `from_value` needs an id
        // only to derive the adapter_id that this shape-only path discards;
        // cross-field checks involving the real extension id belong in
        // `project_product_adapter_sections` below.
        let placeholder = ExtensionId::new("x").map_err(|e| e.to_string())?;
        ProductAdapterHostApiSection::from_value(
            &placeholder,
            host_api.section.clone(),
            section.clone(),
        )
        .map(|_| ())
        .map_err(|e| e.to_string())
    }
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RegistryError {
    #[error(transparent)]
    Manifest(#[from] ManifestV2Error),
    #[error("invalid {field}: {reason}")]
    InvalidValue { field: &'static str, reason: String },
    #[error("product adapter manifest section {section} parse failed: {reason}")]
    ManifestSectionParse {
        section: ManifestSectionPath,
        reason: String,
    },
    #[error("inline secret material is not allowed in manifest field {field}")]
    InlineSecretMaterial { field: String },
    #[error("duplicate credential handle {handle}")]
    DuplicateCredentialHandle { handle: EgressCredentialHandle },
    #[error("duplicate credential binding {handle}")]
    DuplicateCredentialBinding { handle: EgressCredentialHandle },
    #[error("duplicate egress target")]
    DuplicateEgressTarget,
    #[error("egress references undeclared credential handle {handle}")]
    UndeclaredEgressCredentialHandle { handle: EgressCredentialHandle },
    #[error("installation references unknown extension manifest {extension_id}")]
    UnknownManifest { extension_id: ExtensionId },
    #[error("installation binds undeclared credential handle {handle}")]
    UndeclaredCredentialHandle { handle: EgressCredentialHandle },
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
}

// ---------------------------------------------------------------------------
// Internal validation helpers
// ---------------------------------------------------------------------------

fn validate_installation_against_manifest(
    manifests: &HashMap<ExtensionId, ExtensionManifestRecord>,
    installation: &ExtensionInstallation,
) -> Result<(), RegistryError> {
    let manifest = manifests.get(installation.extension_id()).ok_or_else(|| {
        RegistryError::UnknownManifest {
            extension_id: installation.extension_id().clone(),
        }
    })?;
    validate_installation_against_one_manifest(manifest, installation)
}

fn validate_installation_against_one_manifest(
    manifest: &ExtensionManifestRecord,
    installation: &ExtensionInstallation,
) -> Result<(), RegistryError> {
    if manifest.extension_id() != installation.manifest_ref().extension_id() {
        return Err(RegistryError::ManifestExtensionMismatch {
            extension_id: installation.extension_id().clone(),
            manifest_extension_id: installation.manifest_ref().extension_id().clone(),
        });
    }
    match (
        manifest.manifest_hash(),
        installation.manifest_ref().manifest_hash(),
    ) {
        (Some(registered), Some(referenced)) if registered != referenced => {
            return Err(RegistryError::ManifestHashMismatch {
                extension_id: installation.extension_id().clone(),
            });
        }
        (Some(_), None) | (None, Some(_)) => {
            return Err(RegistryError::ManifestHashMismatch {
                extension_id: installation.extension_id().clone(),
            });
        }
        _ => {}
    }

    // Credential bindings must only reference handles declared in the manifest.
    // For non-product-adapter manifests there are no declared credentials, so
    // any binding would be invalid; but installations on non-PA manifests are
    // allowed (they just must not carry bindings).
    let declared: BTreeSet<_> = manifest
        .product_adapters()
        .iter()
        .flat_map(|pa| pa.required_credentials().iter().cloned())
        .collect();
    for binding in installation.credential_bindings() {
        if !declared.contains(binding.credential_handle()) {
            return Err(RegistryError::UndeclaredCredentialHandle {
                handle: binding.credential_handle().clone(),
            });
        }
    }
    Ok(())
}

fn validate_bindings_unique(
    credential_bindings: &[ExtensionCredentialBinding],
) -> Result<(), RegistryError> {
    let mut seen = BTreeSet::new();
    for binding in credential_bindings {
        if !seen.insert(binding.credential_handle.clone()) {
            return Err(RegistryError::DuplicateCredentialBinding {
                handle: binding.credential_handle.clone(),
            });
        }
    }
    Ok(())
}

fn validate_nonempty_noncontrol(field: &'static str, value: &str) -> Result<(), RegistryError> {
    if value.is_empty() {
        return Err(RegistryError::InvalidValue {
            field,
            reason: "must not be empty".to_string(),
        });
    }
    if value.chars().any(|c| c == '\0' || c.is_control()) {
        return Err(RegistryError::InvalidValue {
            field,
            reason: "must not contain control characters".to_string(),
        });
    }
    Ok(())
}

fn validate_auth_requirement(requirement: &AuthRequirement) -> Result<(), RegistryError> {
    match requirement {
        AuthRequirement::RequestSignature {
            header_name,
            timestamp_header_name,
        } => {
            validate_http_token("auth.header_name", header_name)?;
            if let Some(t) = timestamp_header_name.as_deref() {
                validate_http_token("auth.timestamp_header_name", t)?;
            }
        }
        AuthRequirement::SharedSecretHeader { header_name } => {
            validate_http_token("auth.header_name", header_name)?;
        }
        AuthRequirement::SessionCookie { name } => {
            validate_http_token("auth.name", name)?;
        }
        AuthRequirement::BearerToken => {}
    }
    Ok(())
}

fn validate_http_token(field: &'static str, value: &str) -> Result<(), RegistryError> {
    if value.is_empty() {
        return Err(RegistryError::InvalidValue {
            field,
            reason: "must not be empty".to_string(),
        });
    }
    for c in value.chars() {
        if !is_http_tchar(c) {
            return Err(RegistryError::InvalidValue {
                field,
                reason: format!(
                    "must be an RFC 7230 token (no CTL, whitespace, or separators); got {value:?}"
                ),
            });
        }
    }
    Ok(())
}

fn is_http_tchar(c: char) -> bool {
    matches!(
        c,
        '!' | '#' | '$' | '%' | '&' | '\'' | '*' | '+' | '-' | '.' | '^' | '_' | '`' | '|' | '~'
    ) || c.is_ascii_alphanumeric()
}

fn reject_inline_secret_material_value(
    path: &str,
    value: &toml::Value,
) -> Result<(), RegistryError> {
    match value {
        toml::Value::Table(table) => {
            for (key, value) in table {
                let child_path = format!("{path}.{key}");
                if is_secret_key_name(key) {
                    return Err(RegistryError::InlineSecretMaterial { field: child_path });
                }
                reject_inline_secret_material_value(&child_path, value)?;
            }
        }
        toml::Value::Array(values) => {
            for (index, value) in values.iter().enumerate() {
                reject_inline_secret_material_value(&format!("{path}[{index}]"), value)?;
            }
        }
        toml::Value::String(value) if looks_like_inline_secret(value) => {
            return Err(RegistryError::InlineSecretMaterial {
                field: path.to_string(),
            });
        }
        _ => {}
    }
    Ok(())
}

fn is_secret_key_name(key: &str) -> bool {
    let normalised: String = key
        .chars()
        .map(|c| {
            if c == '-' {
                '_'
            } else {
                c.to_ascii_lowercase()
            }
        })
        .collect();
    matches!(
        normalised.as_str(),
        "secret"
            | "secrets"
            | "secret_value"
            | "client_secret"
            | "webhook_secret"
            | "token"
            | "raw_token"
            | "access_token"
            | "refresh_token"
            | "bearer_token"
            | "oauth_token"
            | "auth_token"
            | "id_token"
            | "api_key"
            | "apikey"
            | "api_secret"
            | "private_key"
            | "password"
            | "passphrase"
    )
}

fn looks_like_inline_secret(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    if lower.starts_with("sha256:") {
        return false;
    }
    const PREFIXES: &[&str] = &[
        "sk-",   // OpenAI / Anthropic style API keys.
        "xoxb-", // Slack bot token.
        "xoxa-", // Slack app token.
        "xoxp-", // Slack user token.
        "xoxs-", // Slack service token.
        "xoxe-", // Slack configuration token.
        "ghp_",  // GitHub personal access token.
        "gho_",  // GitHub OAuth token.
        "ghu_",  // GitHub user-to-server token.
        "ghs_",  // GitHub server-to-server token.
        "ghr_",  // GitHub refresh token.
    ];
    PREFIXES.iter().any(|p| lower.starts_with(p))
        || looks_like_aws_access_key(value)
        || lower.contains("begin private key")
        || lower.contains("begin rsa private key")
        || (value.len() >= 30 && value.starts_with("eyJ") && value.contains('.'))
        || has_uri_userinfo(value)
        || looks_like_telegram_token(value)
}

fn looks_like_aws_access_key(value: &str) -> bool {
    if value.len() != 20 {
        return false;
    }
    let Some(prefix) = value.get(..4) else {
        return false;
    };
    (prefix.eq_ignore_ascii_case("AKIA") || prefix.eq_ignore_ascii_case("ASIA"))
        && value[4..]
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
}

fn has_uri_userinfo(value: &str) -> bool {
    let Some((_, rest)) = value.split_once("://") else {
        return false;
    };
    rest.split('/').next().unwrap_or_default().contains('@')
}

fn looks_like_telegram_token(value: &str) -> bool {
    let Some((prefix, suffix)) = value.split_once(':') else {
        return false;
    };
    prefix.len() >= 6
        && prefix.chars().all(|c| c.is_ascii_digit())
        && suffix.len() >= 10
        && suffix
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn project_product_adapter_sections(
    raw_toml: &str,
    manifest: &ExtensionManifestV2,
) -> Result<Vec<ProductAdapterHostApiSection>, RegistryError> {
    // Safety: PRODUCT_ADAPTER_SECTION_PREFIX is a non-empty, control-char-free
    // ASCII identifier defined as a module constant.
    let root_section = ManifestSectionPath::new(PRODUCT_ADAPTER_SECTION_PREFIX)
        .map_err(RegistryError::Manifest)?;
    // `ironclaw_extensions` validates host-api sections from its internal
    // TOML section table but does not expose that table as a public projection
    // API. Re-parse here so this crate can build typed ProductAdapter entries
    // without reaching through the manifest parser's private representation.
    let value: toml::Value =
        toml::from_str(raw_toml).map_err(|error| RegistryError::ManifestSectionParse {
            section: root_section.clone(),
            reason: error.to_string(),
        })?;
    let mut sections = Vec::new();
    for host_api in &manifest.host_apis {
        if host_api.id.as_str() != PRODUCT_ADAPTER_HOST_API_ID {
            continue;
        }
        let section_value = section_value(&value, &host_api.section)?;
        sections.push(ProductAdapterHostApiSection::from_value(
            &manifest.id,
            host_api.section.clone(),
            section_value.clone(),
        )?);
    }
    Ok(sections)
}

fn section_value<'a>(
    root: &'a toml::Value,
    path: &ManifestSectionPath,
) -> Result<&'a toml::Value, RegistryError> {
    let mut current = root;
    for segment in path.as_str().split('.') {
        current = current
            .as_table()
            .and_then(|table| table.get(segment))
            .ok_or_else(|| RegistryError::ManifestSectionParse {
                section: path.clone(),
                reason: "section path does not exist".to_string(),
            })?;
    }
    Ok(current)
}

// ---------------------------------------------------------------------------
// Raw deserialization shapes for ProductAdapter section
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawProductAdapterSection {
    surface_kind: ProductSurfaceKind,
    auth: RawProductAdapterAuth,
    capabilities: RawProductAdapterCapabilities,
    #[serde(default)]
    required_credentials: Vec<RawProductAdapterCredential>,
    #[serde(default)]
    egress: Vec<DeclaredEgressTarget>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawProductAdapterCapabilities {
    flags: Vec<ProductCapabilityFlag>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawProductAdapterCredential {
    handle: EgressCredentialHandle,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum RawProductAdapterAuth {
    RequestSignature {
        header_name: String,
        #[serde(default)]
        timestamp_header_name: Option<String>,
    },
    SharedSecretHeader {
        header_name: String,
    },
    SessionCookie {
        name: String,
    },
    BearerToken,
}

impl RawProductAdapterAuth {
    fn into_auth_requirement(self) -> Result<AuthRequirement, RegistryError> {
        let requirement = match self {
            Self::RequestSignature {
                header_name,
                timestamp_header_name,
            } => AuthRequirement::RequestSignature {
                header_name,
                timestamp_header_name,
            },
            Self::SharedSecretHeader { header_name } => {
                AuthRequirement::SharedSecretHeader { header_name }
            }
            Self::SessionCookie { name } => AuthRequirement::SessionCookie { name },
            Self::BearerToken => AuthRequirement::BearerToken,
        };
        validate_auth_requirement(&requirement)?;
        Ok(requirement)
    }
}
