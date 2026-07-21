use std::collections::BTreeSet;
use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ironclaw_filesystem::{
    CasExpectation, Entry, FilesystemError, Filter, IndexKey, IndexKind, IndexName, IndexSpec,
    IndexValue, Page, RecordKind, RecordVersion, RootFilesystem, VersionedEntry,
};
use ironclaw_host_api::{
    ExtensionId, HostPortCatalog, SecretHandle, UserId, VirtualPath, sha256_digest_token,
};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use thiserror::Error;

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

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ExtensionRemovalCleanupAdapterId(String);

impl ExtensionRemovalCleanupAdapterId {
    pub fn new(value: impl Into<String>) -> Result<Self, ExtensionInstallationError> {
        validate_cleanup_id(value.into(), "cleanup adapter").map(Self)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_inner(self) -> String {
        self.0
    }
}

impl AsRef<str> for ExtensionRemovalCleanupAdapterId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for ExtensionRemovalCleanupAdapterId {
    type Error = ExtensionInstallationError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<ExtensionRemovalCleanupAdapterId> for String {
    fn from(value: ExtensionRemovalCleanupAdapterId) -> Self {
        value.into_inner()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ExtensionRemovalChannelId(String);

impl ExtensionRemovalChannelId {
    pub fn new(value: impl Into<String>) -> Result<Self, ExtensionInstallationError> {
        validate_cleanup_id(value.into(), "cleanup channel").map(Self)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_inner(self) -> String {
        self.0
    }
}

impl AsRef<str> for ExtensionRemovalChannelId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for ExtensionRemovalChannelId {
    type Error = ExtensionInstallationError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<ExtensionRemovalChannelId> for String {
    fn from(value: ExtensionRemovalChannelId) -> Self {
        value.into_inner()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", deny_unknown_fields)]
pub enum ExtensionRemovalCleanupBinding {
    ChannelConnection { channel: ExtensionRemovalChannelId },
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtensionRemovalCleanupRequirement {
    pub adapter_id: ExtensionRemovalCleanupAdapterId,
    pub binding: ExtensionRemovalCleanupBinding,
}

impl ExtensionRemovalCleanupRequirement {
    pub fn channel_connection(
        adapter_id: ExtensionRemovalCleanupAdapterId,
        channel: ExtensionRemovalChannelId,
    ) -> Self {
        Self {
            adapter_id,
            binding: ExtensionRemovalCleanupBinding::ChannelConnection { channel },
        }
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
    removal_cleanup_requirements: Vec<ExtensionRemovalCleanupRequirement>,
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
            removal_cleanup_requirements: Vec::new(),
        })
    }

    /// Attach host-trusted declarative cleanup metadata to the persisted
    /// manifest record. These requirements are never parsed from extension
    /// supplied TOML; catalog construction is the only production writer.
    pub fn with_removal_cleanup_requirements(
        mut self,
        requirements: Vec<ExtensionRemovalCleanupRequirement>,
    ) -> Self {
        self.removal_cleanup_requirements = requirements;
        self
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

    pub fn removal_cleanup_requirements(&self) -> &[ExtensionRemovalCleanupRequirement] {
        &self.removal_cleanup_requirements
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
        f.write_str(&self.0)
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

/// Who an installation belongs to (#5459 P1 — shared vs member installs,
/// membership model per the 2026-07-08 pivot in
/// `docs/plans/2026-07-01-private-tool-installs.md`).
///
/// `Tenant` = installed for the whole tenant (the historical behavior, and
/// what an operator install produces): every user sees and can dispatch the
/// extension's capabilities. `Users` = held by a set of members: any number
/// of users can independently install the same tool; only members see it,
/// only members get grants minted for it.
///
/// Legacy persisted records predate this field and deserialize as `Tenant`
/// via `#[serde(default)]` — no migration required, no behavior change for
/// existing installs. Rows written by the slot iteration of this feature
/// (`{"kind": "user", "user_id": …}`) deserialize as a singleton member set.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum InstallationOwner {
    #[default]
    Tenant,
    Users {
        user_ids: BTreeSet<UserId>,
    },
}

impl InstallationOwner {
    /// Singleton member set — what a single member's install produces.
    pub fn user(user_id: UserId) -> Self {
        Self::Users {
            user_ids: BTreeSet::from([user_id]),
        }
    }

    /// Member set; rejects an empty set (an installation must belong to the
    /// tenant or to at least one member — an empty set would be a row nobody
    /// can see, operate, or remove).
    pub fn users(user_ids: BTreeSet<UserId>) -> Result<Self, ExtensionInstallationError> {
        if user_ids.is_empty() {
            return Err(ExtensionInstallationError::EmptyOwnerMembers);
        }
        Ok(Self::Users { user_ids })
    }

    pub fn is_tenant(&self) -> bool {
        matches!(self, Self::Tenant)
    }

    /// The member set, if the installation is member-held.
    pub fn members(&self) -> Option<&BTreeSet<UserId>> {
        match self {
            Self::Users { user_ids } => Some(user_ids),
            Self::Tenant => None,
        }
    }

    /// Whether `caller` may see/use this installation: tenant-wide entries
    /// are visible to everyone, member-held entries only to their members.
    pub fn visible_to(&self, caller: &UserId) -> bool {
        match self {
            Self::Tenant => true,
            Self::Users { user_ids } => user_ids.contains(caller),
        }
    }
}

impl Serialize for InstallationOwner {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        InstallationOwnerWire::from(self.clone()).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for InstallationOwner {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        InstallationOwnerWire::deserialize(deserializer)?
            .try_into()
            .map_err(serde::de::Error::custom)
    }
}

/// Wire shape of [`InstallationOwner`]. `user` is the read-only legacy kind
/// written by the slot iteration of #5459 P1 (a single owning user); it folds
/// into a singleton member set on load and is never written back.
#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum InstallationOwnerWire {
    Tenant,
    User { user_id: UserId },
    Users { user_ids: BTreeSet<UserId> },
}

impl From<InstallationOwner> for InstallationOwnerWire {
    fn from(owner: InstallationOwner) -> Self {
        match owner {
            InstallationOwner::Tenant => Self::Tenant,
            InstallationOwner::Users { user_ids } => Self::Users { user_ids },
        }
    }
}

impl TryFrom<InstallationOwnerWire> for InstallationOwner {
    type Error = ExtensionInstallationError;

    fn try_from(wire: InstallationOwnerWire) -> Result<Self, Self::Error> {
        match wire {
            InstallationOwnerWire::Tenant => Ok(Self::Tenant),
            InstallationOwnerWire::User { user_id } => Ok(Self::user(user_id)),
            InstallationOwnerWire::Users { user_ids } => Self::users(user_ids),
        }
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
    // Tenant-owned rows serialize WITHOUT the field so they keep the exact
    // pre-#5459 byte shape: a rollback to an older binary (whose wire struct
    // uses `deny_unknown_fields`) still loads a state.json that contains no
    // private installs. Only user-private rows carry the field — the one
    // shape an older binary genuinely cannot represent.
    #[serde(default, skip_serializing_if = "InstallationOwner::is_tenant")]
    owner: InstallationOwner,
}

/// All persisted fields needed to reconstruct an installation without
/// inventing fresh health or timestamp state.
#[derive(Debug)]
pub struct ExtensionInstallationPersistedParts {
    pub installation_id: ExtensionInstallationId,
    pub extension_id: ExtensionId,
    pub activation_state: ExtensionActivationState,
    pub manifest_ref: ExtensionManifestRef,
    pub credential_bindings: Vec<ExtensionCredentialBinding>,
    pub health: ExtensionHealthSnapshot,
    pub updated_at: DateTime<Utc>,
    pub owner: InstallationOwner,
}

impl ExtensionInstallation {
    pub fn new(
        installation_id: ExtensionInstallationId,
        extension_id: ExtensionId,
        activation_state: ExtensionActivationState,
        manifest_ref: ExtensionManifestRef,
        credential_bindings: Vec<ExtensionCredentialBinding>,
        updated_at: DateTime<Utc>,
        owner: InstallationOwner,
    ) -> Result<Self, ExtensionInstallationError> {
        Self::from_persisted_parts(ExtensionInstallationPersistedParts {
            installation_id,
            extension_id,
            activation_state,
            manifest_ref,
            credential_bindings,
            health: ExtensionHealthSnapshot::new(ExtensionHealthStatus::Healthy, None, updated_at),
            updated_at,
            owner,
        })
    }

    /// Reconstruct an installation with all state read from persistence.
    ///
    /// The ordinary [`Self::new`] constructor starts a fresh installation with
    /// a healthy snapshot. Persistence adapters use this neutral constructor
    /// when they need to preserve an existing health snapshot and timestamp
    /// while changing the canonical installation identity.
    pub fn from_persisted_parts(
        parts: ExtensionInstallationPersistedParts,
    ) -> Result<Self, ExtensionInstallationError> {
        if parts.manifest_ref.extension_id() != &parts.extension_id {
            return Err(ExtensionInstallationError::ManifestExtensionMismatch {
                extension_id: parts.extension_id,
                manifest_extension_id: parts.manifest_ref.extension_id().clone(),
            });
        }
        validate_bindings_unique(&parts.credential_bindings)?;
        Ok(Self {
            installation_id: parts.installation_id,
            extension_id: parts.extension_id,
            activation_state: parts.activation_state,
            manifest_ref: parts.manifest_ref,
            credential_bindings: parts.credential_bindings,
            health: parts.health,
            updated_at: parts.updated_at,
            owner: parts.owner,
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

    pub fn owner(&self) -> &InstallationOwner {
        &self.owner
    }

    /// Same installation with a replaced owner (membership join/leave and
    /// operator eviction-to-tenant are single row rewrites); refreshes
    /// `updated_at` like every other row mutation.
    pub fn with_owner(mut self, owner: InstallationOwner) -> Self {
        self.owner = owner;
        self.updated_at = Utc::now();
        self
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
            // Legacy records predate the owner field; they were all
            // tenant-visible, so absent == Tenant is behavior-preserving.
            #[serde(default)]
            owner: InstallationOwner,
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
            owner: wire.owner,
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

    async fn upsert_manifest_and_installation(
        &self,
        manifest: ExtensionManifestRecord,
        installation: ExtensionInstallation,
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

    async fn upsert_manifest_and_installation(
        &self,
        manifest: ExtensionManifestRecord,
        installation: ExtensionInstallation,
    ) -> Result<(), ExtensionInstallationError> {
        (**self)
            .upsert_manifest_and_installation(manifest, installation)
            .await
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

const DEFAULT_INSTALLATION_STATE_PATH: &str = "/system/extensions/.installations";
const MANIFEST_RECORD_KIND: &str = "extension_manifest_record";
const INSTALLATION_RECORD_KIND: &str = "extension_installation_record";
const FILESYSTEM_CAS_RETRIES: usize = 5;

/// Filesystem-backed extension installation state store.
///
/// Manifests and installations are persisted as separate record rows under the
/// configured root path. Secondary indexes are declared on the row prefixes so
/// scans that gate lifecycle behavior can use the filesystem query API instead
/// of loading a monolithic state snapshot.
pub struct FilesystemExtensionInstallationStore {
    filesystem: Arc<dyn RootFilesystem>,
    root: VirtualPath,
    host_ports: HostPortCatalog,
    contracts: HostApiContractRegistry,
    cas_retries: usize,
}

impl fmt::Debug for FilesystemExtensionInstallationStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FilesystemExtensionInstallationStore")
            .field("root", &self.root)
            .field("cas_retries", &self.cas_retries)
            .finish_non_exhaustive()
    }
}

impl FilesystemExtensionInstallationStore {
    pub async fn load_at(
        filesystem: Arc<dyn RootFilesystem>,
        root: VirtualPath,
        host_ports: HostPortCatalog,
        contracts: HostApiContractRegistry,
    ) -> Result<Self, ExtensionInstallationError> {
        let store = Self {
            filesystem,
            root,
            host_ports,
            contracts,
            cas_retries: FILESYSTEM_CAS_RETRIES,
        };
        store.ensure_indexes().await?;
        Ok(store)
    }

    pub fn default_state_path() -> Result<VirtualPath, ExtensionInstallationError> {
        VirtualPath::new(DEFAULT_INSTALLATION_STATE_PATH).map_err(invalid_installation_error)
    }

    pub fn with_cas_retries(mut self, cas_retries: usize) -> Self {
        self.cas_retries = cas_retries;
        self
    }

    async fn ensure_indexes(&self) -> Result<(), ExtensionInstallationError> {
        self.ensure_exact_index(
            &self.manifests_root()?,
            "extension_manifests_by_extension_id",
            "extension_id",
        )
        .await?;
        self.ensure_exact_index(
            &self.installations_root()?,
            "extension_installations_by_installation_id",
            "installation_id",
        )
        .await?;
        self.ensure_exact_index(
            &self.installations_root()?,
            "extension_installations_by_extension_id",
            "extension_id",
        )
        .await?;
        self.ensure_exact_index(
            &self.installations_root()?,
            "extension_installations_by_activation_state",
            "activation_state",
        )
        .await
    }

    async fn ensure_exact_index(
        &self,
        prefix: &VirtualPath,
        name: &'static str,
        key: &'static str,
    ) -> Result<(), ExtensionInstallationError> {
        let name = index_name(name)?;
        let key = index_key(key)?;
        let spec = IndexSpec::new(name, vec![key], IndexKind::Exact);
        self.filesystem
            .ensure_index(prefix, &spec)
            .await
            .map_err(store_unavailable(
                "ensure extension installation store index",
            ))
    }

    fn manifests_root(&self) -> Result<VirtualPath, ExtensionInstallationError> {
        child_path(&self.root, "manifests")
    }

    fn installations_root(&self) -> Result<VirtualPath, ExtensionInstallationError> {
        child_path(&self.root, "installations")
    }

    fn manifest_path(
        &self,
        extension_id: &ExtensionId,
    ) -> Result<VirtualPath, ExtensionInstallationError> {
        child_path(
            &self.manifests_root()?,
            &format!("{}.json", row_token(extension_id.as_str())),
        )
    }

    fn installation_path(
        &self,
        installation_id: &ExtensionInstallationId,
    ) -> Result<VirtualPath, ExtensionInstallationError> {
        child_path(
            &self.installations_root()?,
            &format!("{}.json", row_token(installation_id.as_str())),
        )
    }

    async fn load_manifest_entry(
        &self,
        extension_id: &ExtensionId,
    ) -> Result<Option<(ExtensionManifestRecord, RecordVersion)>, ExtensionInstallationError> {
        let path = self.manifest_path(extension_id)?;
        let Some(entry) = self
            .filesystem
            .get(&path)
            .await
            .map_err(store_unavailable("load extension manifest row"))?
        else {
            return Ok(None);
        };
        let manifest = self.parse_manifest_entry(entry.entry, &path)?;
        if manifest.extension_id() != extension_id {
            return Err(invalid_installation_error(format!(
                "extension manifest row key {extension_id} contained manifest {}",
                manifest.extension_id()
            )));
        }
        Ok(Some((manifest, entry.version)))
    }

    async fn load_installation_entry(
        &self,
        installation_id: &ExtensionInstallationId,
    ) -> Result<Option<(ExtensionInstallation, RecordVersion)>, ExtensionInstallationError> {
        let path = self.installation_path(installation_id)?;
        let Some(entry) = self
            .filesystem
            .get(&path)
            .await
            .map_err(store_unavailable("load extension installation row"))?
        else {
            return Ok(None);
        };
        let installation = parse_installation_entry(entry.entry, &path)?;
        if installation.installation_id() != installation_id {
            return Err(invalid_installation_error(format!(
                "extension installation row key {installation_id} contained installation {}",
                installation.installation_id()
            )));
        }
        Ok(Some((installation, entry.version)))
    }

    async fn query_installations_by_extension(
        &self,
        extension_id: &ExtensionId,
    ) -> Result<Vec<ExtensionInstallation>, ExtensionInstallationError> {
        let filter = Filter::Eq {
            key: index_key("extension_id")?,
            value: IndexValue::Text(extension_id.as_str().to_string()),
        };
        self.query_installations(&filter).await
    }

    async fn query_installations(
        &self,
        filter: &Filter,
    ) -> Result<Vec<ExtensionInstallation>, ExtensionInstallationError> {
        let rows = query_all(&self.filesystem, &self.installations_root()?, filter).await?;
        rows.into_iter()
            .map(|entry| parse_installation_entry(entry.entry, &entry.path))
            .collect()
    }

    async fn query_manifests(
        &self,
        filter: &Filter,
    ) -> Result<Vec<ExtensionManifestRecord>, ExtensionInstallationError> {
        let rows = query_all(&self.filesystem, &self.manifests_root()?, filter).await?;
        rows.into_iter()
            .map(|entry| self.parse_manifest_entry(entry.entry, &entry.path))
            .collect()
    }

    fn parse_manifest_entry(
        &self,
        entry: Entry,
        path: &VirtualPath,
    ) -> Result<ExtensionManifestRecord, ExtensionInstallationError> {
        ensure_entry_kind(&entry, MANIFEST_RECORD_KIND, path)?;
        let wire: WireManifestRecord = entry
            .parse_json()
            .map_err(|error| corrupt_row("deserialize extension manifest row", path, error))?;
        wire.into_manifest_record(&self.host_ports, &self.contracts)
    }

    async fn put_manifest(
        &self,
        manifest: &ExtensionManifestRecord,
        cas: CasExpectation,
    ) -> Result<(), SaveRowError> {
        let path = self.manifest_path(manifest.extension_id())?;
        match self
            .filesystem
            .put(&path, entry_for_manifest(manifest)?, cas)
            .await
        {
            Ok(_) => Ok(()),
            Err(FilesystemError::VersionMismatch { .. }) => Err(SaveRowError::CasConflict),
            Err(error) => Err(SaveRowError::Store(store_unavailable(
                "save extension manifest row",
            )(error))),
        }
    }

    async fn put_installation(
        &self,
        installation: &ExtensionInstallation,
        cas: CasExpectation,
    ) -> Result<(), SaveRowError> {
        let path = self.installation_path(installation.installation_id())?;
        match self
            .filesystem
            .put(&path, entry_for_installation(installation)?, cas)
            .await
        {
            Ok(_) => Ok(()),
            Err(FilesystemError::VersionMismatch { .. }) => Err(SaveRowError::CasConflict),
            Err(error) => Err(SaveRowError::Store(store_unavailable(
                "save extension installation row",
            )(error))),
        }
    }

    async fn delete_installation_row(
        &self,
        installation_id: &ExtensionInstallationId,
        version: RecordVersion,
    ) -> Result<(), SaveRowError> {
        let path = self.installation_path(installation_id)?;
        match self.filesystem.delete_if_version(&path, version).await {
            Ok(()) => Ok(()),
            Err(FilesystemError::VersionMismatch { .. }) => Err(SaveRowError::CasConflict),
            Err(FilesystemError::NotFound { .. }) => Err(SaveRowError::NotFound),
            Err(error) => Err(SaveRowError::Store(store_unavailable(
                "delete extension installation row",
            )(error))),
        }
    }

    async fn delete_manifest_row(
        &self,
        extension_id: &ExtensionId,
        version: RecordVersion,
    ) -> Result<(), SaveRowError> {
        let path = self.manifest_path(extension_id)?;
        match self.filesystem.delete_if_version(&path, version).await {
            Ok(()) => Ok(()),
            Err(FilesystemError::VersionMismatch { .. }) => Err(SaveRowError::CasConflict),
            Err(FilesystemError::NotFound { .. }) => Err(SaveRowError::NotFound),
            Err(error) => Err(SaveRowError::Store(store_unavailable(
                "delete extension manifest row",
            )(error))),
        }
    }

    async fn restore_manifest_best_effort(&self, prior: Option<ExtensionManifestRecord>) {
        if let Some(prior) = prior
            && let Err(error) = self.put_manifest(&prior, CasExpectation::Any).await
        {
            let _ = error;
        }
    }
}

#[async_trait]
impl ExtensionInstallationStore for FilesystemExtensionInstallationStore {
    async fn list_manifests(
        &self,
    ) -> Result<Vec<ExtensionManifestRecord>, ExtensionInstallationError> {
        let mut manifests = self.query_manifests(&Filter::All).await?;
        manifests.sort_by(|a, b| a.extension_id().cmp(b.extension_id()));
        Ok(manifests)
    }

    async fn get_manifest(
        &self,
        extension_id: &ExtensionId,
    ) -> Result<Option<ExtensionManifestRecord>, ExtensionInstallationError> {
        self.load_manifest_entry(extension_id)
            .await
            .map(|entry| entry.map(|(manifest, _)| manifest))
    }

    async fn upsert_manifest(
        &self,
        manifest: ExtensionManifestRecord,
    ) -> Result<(), ExtensionInstallationError> {
        for installation in self
            .query_installations_by_extension(manifest.extension_id())
            .await?
        {
            validate_installation_against_one_manifest(&manifest, &installation)?;
        }
        self.put_manifest(&manifest, CasExpectation::Any)
            .await
            .map_err(SaveRowError::into_installation_error)
    }

    async fn upsert_manifest_and_installation(
        &self,
        manifest: ExtensionManifestRecord,
        installation: ExtensionInstallation,
    ) -> Result<(), ExtensionInstallationError> {
        validate_installation_against_one_manifest(&manifest, &installation)?;
        let extension_id = manifest.extension_id().clone();
        let prior_manifest = self.get_manifest(&extension_id).await?;
        self.put_manifest(&manifest, CasExpectation::Any)
            .await
            .map_err(SaveRowError::into_installation_error)?;
        if let Err(error) = self
            .put_installation(&installation, CasExpectation::Any)
            .await
        {
            self.restore_manifest_best_effort(prior_manifest).await;
            return Err(error.into_installation_error());
        }
        Ok(())
    }

    async fn list_installations(
        &self,
    ) -> Result<Vec<ExtensionInstallation>, ExtensionInstallationError> {
        let mut installations = self.query_installations(&Filter::All).await?;
        installations.sort_by(|a, b| a.installation_id().cmp(b.installation_id()));
        Ok(installations)
    }

    async fn list_enabled_installations(
        &self,
    ) -> Result<Vec<ExtensionInstallation>, ExtensionInstallationError> {
        let filter = Filter::Eq {
            key: index_key("activation_state")?,
            value: IndexValue::Text(activation_state_key(ExtensionActivationState::Enabled).into()),
        };
        let mut installations = self.query_installations(&filter).await?;
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
        self.load_installation_entry(installation_id)
            .await
            .map(|entry| entry.map(|(installation, _)| installation))
    }

    async fn upsert_installation(
        &self,
        installation: ExtensionInstallation,
    ) -> Result<(), ExtensionInstallationError> {
        let manifest = self
            .get_manifest(installation.extension_id())
            .await?
            .ok_or_else(|| ExtensionInstallationError::UnknownManifest {
                extension_id: installation.extension_id().clone(),
            })?;
        validate_installation_against_one_manifest(&manifest, &installation)?;
        self.put_installation(&installation, CasExpectation::Any)
            .await
            .map_err(SaveRowError::into_installation_error)
    }

    async fn set_activation_state(
        &self,
        installation_id: &ExtensionInstallationId,
        state: ExtensionActivationState,
    ) -> Result<(), ExtensionInstallationError> {
        for _ in 0..=self.cas_retries {
            let Some((mut installation, version)) =
                self.load_installation_entry(installation_id).await?
            else {
                return Err(ExtensionInstallationError::InstallationNotFound {
                    installation_id: installation_id.clone(),
                });
            };
            if installation.activation_state() == state {
                return Ok(());
            }
            if state == ExtensionActivationState::Enabled {
                let manifest = self
                    .get_manifest(installation.extension_id())
                    .await?
                    .ok_or_else(|| ExtensionInstallationError::UnknownManifest {
                        extension_id: installation.extension_id().clone(),
                    })?;
                validate_installation_against_one_manifest(&manifest, &installation)?;
            }
            installation.set_activation_state(state);
            match self
                .put_installation(&installation, CasExpectation::Version(version))
                .await
            {
                Ok(()) => return Ok(()),
                Err(SaveRowError::CasConflict) => continue,
                Err(error) => return Err(error.into_installation_error()),
            }
        }
        Err(store_unavailable_error(
            "extension installation row changed repeatedly while updating activation state",
        ))
    }

    async fn delete_installation(
        &self,
        installation_id: &ExtensionInstallationId,
    ) -> Result<(), ExtensionInstallationError> {
        for _ in 0..=self.cas_retries {
            let Some((_, version)) = self.load_installation_entry(installation_id).await? else {
                return Err(ExtensionInstallationError::InstallationNotFound {
                    installation_id: installation_id.clone(),
                });
            };
            match self.delete_installation_row(installation_id, version).await {
                Ok(()) => return Ok(()),
                Err(SaveRowError::CasConflict) => continue,
                Err(SaveRowError::NotFound) => {
                    return Err(ExtensionInstallationError::InstallationNotFound {
                        installation_id: installation_id.clone(),
                    });
                }
                Err(error) => return Err(error.into_installation_error()),
            }
        }
        Err(store_unavailable_error(
            "extension installation row changed repeatedly while deleting installation",
        ))
    }

    async fn delete_manifest(
        &self,
        extension_id: &ExtensionId,
    ) -> Result<(), ExtensionInstallationError> {
        if !self
            .query_installations_by_extension(extension_id)
            .await?
            .is_empty()
        {
            return Err(ExtensionInstallationError::InvalidInstallation {
                reason: format!("extension {extension_id} still has installations"),
            });
        }
        for _ in 0..=self.cas_retries {
            let Some((_, version)) = self.load_manifest_entry(extension_id).await? else {
                return Err(ExtensionInstallationError::ManifestNotFound {
                    extension_id: extension_id.clone(),
                });
            };
            match self.delete_manifest_row(extension_id, version).await {
                Ok(()) => return Ok(()),
                Err(SaveRowError::CasConflict) => continue,
                Err(SaveRowError::NotFound) => {
                    return Err(ExtensionInstallationError::ManifestNotFound {
                        extension_id: extension_id.clone(),
                    });
                }
                Err(error) => return Err(error.into_installation_error()),
            }
        }
        Err(store_unavailable_error(
            "extension manifest row changed repeatedly while deleting manifest",
        ))
    }

    async fn update_health(
        &self,
        installation_id: &ExtensionInstallationId,
        health: ExtensionHealthSnapshot,
    ) -> Result<(), ExtensionInstallationError> {
        for _ in 0..=self.cas_retries {
            let Some((mut installation, version)) =
                self.load_installation_entry(installation_id).await?
            else {
                return Err(ExtensionInstallationError::InstallationNotFound {
                    installation_id: installation_id.clone(),
                });
            };
            installation.set_health(health.clone());
            match self
                .put_installation(&installation, CasExpectation::Version(version))
                .await
            {
                Ok(()) => return Ok(()),
                Err(SaveRowError::CasConflict) => continue,
                Err(error) => return Err(error.into_installation_error()),
            }
        }
        Err(store_unavailable_error(
            "extension installation row changed repeatedly while updating health",
        ))
    }
}

#[derive(Debug)]
enum SaveRowError {
    CasConflict,
    NotFound,
    Store(ExtensionInstallationError),
}

impl From<ExtensionInstallationError> for SaveRowError {
    fn from(error: ExtensionInstallationError) -> Self {
        Self::Store(error)
    }
}

impl SaveRowError {
    fn into_installation_error(self) -> ExtensionInstallationError {
        match self {
            Self::CasConflict => store_unavailable_error("extension installation row CAS conflict"),
            Self::NotFound => store_unavailable_error("extension installation row disappeared"),
            Self::Store(error) => error,
        }
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
    fn into_manifest_record(
        self,
        host_ports: &HostPortCatalog,
        contracts: &HostApiContractRegistry,
    ) -> Result<ExtensionManifestRecord, ExtensionInstallationError> {
        ExtensionManifestRecord::from_toml_with_contracts(
            self.raw_toml,
            self.source.into_manifest_source(),
            host_ports,
            self.manifest_hash,
            contracts,
        )
        .map(|record| record.with_removal_cleanup_requirements(self.removal_cleanup_requirements))
    }
}

impl From<&ExtensionManifestRecord> for WireManifestRecord {
    fn from(record: &ExtensionManifestRecord) -> Self {
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

async fn query_all(
    filesystem: &Arc<dyn RootFilesystem>,
    prefix: &VirtualPath,
    filter: &Filter,
) -> Result<Vec<VersionedEntry>, ExtensionInstallationError> {
    let mut out = Vec::new();
    let mut offset: u64 = 0;
    loop {
        let page = Page::new(offset, Page::MAX_LIMIT);
        let rows = filesystem
            .query(prefix, filter, page)
            .await
            .map_err(store_unavailable("query extension installation rows"))?;
        let received = rows.len();
        out.extend(rows);
        if received < Page::MAX_LIMIT as usize {
            break;
        }
        offset = offset.saturating_add(received as u64);
    }
    Ok(out)
}

fn entry_for_manifest(
    manifest: &ExtensionManifestRecord,
) -> Result<Entry, ExtensionInstallationError> {
    let payload = serde_json::to_value(WireManifestRecord::from(manifest))
        .map_err(invalid_installation_error)?;
    Ok(Entry::record(record_kind(MANIFEST_RECORD_KIND)?, &payload)
        .map_err(invalid_installation_error)?
        .with_indexed(
            index_key("extension_id")?,
            IndexValue::Text(manifest.extension_id().as_str().to_string()),
        )
        .with_indexed(
            index_key("manifest_source")?,
            IndexValue::Text(manifest_source_key(manifest.manifest().source).into()),
        ))
}

fn entry_for_installation(
    installation: &ExtensionInstallation,
) -> Result<Entry, ExtensionInstallationError> {
    let payload = serde_json::to_value(installation).map_err(invalid_installation_error)?;
    Ok(
        Entry::record(record_kind(INSTALLATION_RECORD_KIND)?, &payload)
            .map_err(invalid_installation_error)?
            .with_indexed(
                index_key("installation_id")?,
                IndexValue::Text(installation.installation_id().as_str().to_string()),
            )
            .with_indexed(
                index_key("extension_id")?,
                IndexValue::Text(installation.extension_id().as_str().to_string()),
            )
            .with_indexed(
                index_key("activation_state")?,
                IndexValue::Text(activation_state_key(installation.activation_state()).into()),
            ),
    )
}

fn parse_installation_entry(
    entry: Entry,
    path: &VirtualPath,
) -> Result<ExtensionInstallation, ExtensionInstallationError> {
    ensure_entry_kind(&entry, INSTALLATION_RECORD_KIND, path)?;
    entry
        .parse_json()
        .map_err(|error| corrupt_row("deserialize extension installation row", path, error))
}

fn ensure_entry_kind(
    entry: &Entry,
    expected: &'static str,
    path: &VirtualPath,
) -> Result<(), ExtensionInstallationError> {
    match entry.kind.as_ref().map(RecordKind::as_str) {
        Some(actual) if actual == expected => Ok(()),
        _ => Err(invalid_installation_error(format!(
            "extension installation store row at {} had unexpected record kind",
            path.as_str()
        ))),
    }
}

fn child_path(root: &VirtualPath, child: &str) -> Result<VirtualPath, ExtensionInstallationError> {
    VirtualPath::new(format!("{}/{}", root.as_str().trim_end_matches('/'), child))
        .map_err(invalid_installation_error)
}

fn row_token(value: &str) -> String {
    sha256_digest_token(value.as_bytes()).replace(':', "_")
}

fn activation_state_key(state: ExtensionActivationState) -> &'static str {
    match state {
        ExtensionActivationState::Installed => "installed",
        ExtensionActivationState::Disabled => "disabled",
        ExtensionActivationState::Enabled => "enabled",
    }
}

fn manifest_source_key(source: ManifestSource) -> &'static str {
    match source {
        ManifestSource::HostBundled => "host_bundled",
        ManifestSource::InstalledLocal => "installed_local",
        ManifestSource::RegistryInstalled => "registry_installed",
    }
}

fn record_kind(value: &'static str) -> Result<RecordKind, ExtensionInstallationError> {
    RecordKind::new(value).map_err(invalid_installation_error)
}

fn index_name(value: &'static str) -> Result<IndexName, ExtensionInstallationError> {
    IndexName::new(value).map_err(invalid_installation_error)
}

fn index_key(value: &'static str) -> Result<IndexKey, ExtensionInstallationError> {
    IndexKey::new(value).map_err(invalid_installation_error)
}

fn corrupt_row(
    operation: &'static str,
    path: &VirtualPath,
    error: impl fmt::Display,
) -> ExtensionInstallationError {
    let _ = path;
    invalid_installation_error(format!("{operation}: {error}"))
}

fn store_unavailable(
    operation: &'static str,
) -> impl FnOnce(FilesystemError) -> ExtensionInstallationError {
    move |error| {
        let _ = error;
        store_unavailable_error(operation)
    }
}

fn invalid_installation_error(error: impl fmt::Display) -> ExtensionInstallationError {
    ExtensionInstallationError::InvalidInstallation {
        reason: error.to_string(),
    }
}

fn store_unavailable_error(error: impl fmt::Display) -> ExtensionInstallationError {
    ExtensionInstallationError::StoreUnavailable {
        reason: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use ironclaw_filesystem::InMemoryBackend;
    use ironclaw_host_api::{ExtensionId, HostPortCatalog, VirtualPath};

    use super::*;
    use crate::ManifestSource;

    #[test]
    fn removal_cleanup_ids_validate_and_round_trip_their_canonical_wire_values() {
        let adapter = ExtensionRemovalCleanupAdapterId::new("slack.personal")
            .expect("canonical cleanup adapter");
        assert_eq!(adapter.as_str(), "slack.personal");
        assert_eq!(adapter.as_ref(), "slack.personal");
        assert_eq!(adapter.clone().into_inner(), "slack.personal");
        assert_eq!(String::from(adapter.clone()), "slack.personal");
        assert_eq!(
            serde_json::from_str::<ExtensionRemovalCleanupAdapterId>(
                &serde_json::to_string(&adapter).expect("serialize adapter")
            )
            .expect("deserialize adapter"),
            adapter
        );

        let channel = ExtensionRemovalChannelId::new("slack").expect("canonical cleanup channel");
        assert_eq!(channel.as_str(), "slack");
        assert_eq!(channel.as_ref(), "slack");
        assert_eq!(channel.clone().into_inner(), "slack");
        assert_eq!(String::from(channel.clone()), "slack");
        assert_eq!(
            serde_json::from_str::<ExtensionRemovalChannelId>(
                &serde_json::to_string(&channel).expect("serialize channel")
            )
            .expect("deserialize channel"),
            channel
        );

        for invalid in ["", "Slack", "slack/connection", "-slack", "slack-"] {
            let wire = serde_json::to_string(invalid).expect("serialize invalid cleanup id");
            assert!(
                serde_json::from_str::<ExtensionRemovalCleanupAdapterId>(&wire).is_err(),
                "invalid cleanup adapter must be rejected: {invalid}"
            );
            assert!(
                serde_json::from_str::<ExtensionRemovalChannelId>(&wire).is_err(),
                "invalid cleanup channel must be rejected: {invalid}"
            );
        }
    }

    #[tokio::test]
    async fn delete_manifest_rejects_active_installations() {
        let store = filesystem_store().await;
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

    async fn filesystem_store() -> FilesystemExtensionInstallationStore {
        FilesystemExtensionInstallationStore::load_at(
            Arc::new(InMemoryBackend::new()),
            VirtualPath::new("/system/extensions/.installations/test").expect("valid root"),
            HostPortCatalog::empty(),
            HostApiContractRegistry::new(),
        )
        .await
        .expect("filesystem store")
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
            InstallationOwner::Tenant,
        )
        .expect("installation")
    }

    /// #5459 P1: legacy persisted rows predate the `owner` field and were all
    /// tenant-visible; a record without it MUST deserialize as `Tenant` (no
    /// migration). The inverse also holds: a TENANT-owned record serializes
    /// WITHOUT the field, keeping the exact pre-#5459 byte shape so a rollback
    /// to an older binary (`deny_unknown_fields` wire struct) still loads a
    /// state.json holding no private installs. A user-owned record must
    /// round-trip its owner.
    #[test]
    fn installation_owner_defaults_to_tenant_for_legacy_rows_and_round_trips() {
        let current = installation("fixture", Some("hash-1"));
        let json = serde_json::to_value(&current).expect("serialize installation");
        assert!(
            json.get("owner").is_none(),
            "tenant-owned rows must keep the pre-#5459 shape (rollback compat): {json}"
        );
        let legacy: ExtensionInstallation =
            serde_json::from_value(json).expect("legacy row without owner deserializes");
        assert_eq!(legacy.owner(), &InstallationOwner::Tenant);

        let alice = ironclaw_host_api::UserId::new("alice").expect("user id");
        let private = ExtensionInstallation::new(
            ExtensionInstallationId::new("fixture".to_string()).expect("installation id"),
            ExtensionId::new("fixture".to_string()).expect("extension id"),
            ExtensionActivationState::Installed,
            ExtensionManifestRef::new(ExtensionId::new("fixture".to_string()).unwrap(), None),
            Vec::new(),
            Utc::now(),
            InstallationOwner::user(alice.clone()),
        )
        .expect("installation");
        let json = serde_json::to_string(&private).expect("serialize");
        assert!(
            json.contains(r#""kind":"users""#),
            "member-held rows serialize the set shape: {json}"
        );
        let restored: ExtensionInstallation = serde_json::from_str(&json).expect("round-trip");
        assert!(restored.owner().visible_to(&alice));
        assert_eq!(
            restored.owner().members().map(BTreeSet::len),
            Some(1),
            "singleton member set round-trips"
        );
    }

    /// Membership pivot (2026-07-08): rows written by the slot iteration
    /// carry `{"kind": "user", "user_id": …}` — they MUST keep loading, as a
    /// singleton member set; an empty member set is rejected on the wire and
    /// at construction (a row nobody could see, operate, or remove).
    #[test]
    fn slot_iteration_user_owner_rows_load_as_singleton_member_set() {
        let alice = ironclaw_host_api::UserId::new("alice").expect("user id");
        let bob = ironclaw_host_api::UserId::new("bob").expect("user id");
        let legacy: InstallationOwner =
            serde_json::from_str(r#"{"kind":"user","user_id":"alice"}"#)
                .expect("slot-iteration owner row loads");
        assert!(legacy.visible_to(&alice));
        assert!(!legacy.visible_to(&bob));
        assert_eq!(legacy, InstallationOwner::user(alice.clone()));

        let set: InstallationOwner =
            serde_json::from_str(r#"{"kind":"users","user_ids":["alice","bob"]}"#)
                .expect("member set loads");
        assert!(set.visible_to(&alice) && set.visible_to(&bob));

        serde_json::from_str::<InstallationOwner>(r#"{"kind":"users","user_ids":[]}"#)
            .expect_err("empty member set is rejected on the wire");
        InstallationOwner::users(BTreeSet::new()).expect_err("empty member set is unconstructable");
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
    #[error("installation owner member set must not be empty")]
    EmptyOwnerMembers,
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
    /// The backing installation store could not serve the operation
    /// (IO/backend failure). Retryable, unlike the malformed-request
    /// variants: callers map this to their transient error class (#4091).
    #[error("extension installation store unavailable: {reason}")]
    StoreUnavailable { reason: String },
    #[error("duplicate credential binding {handle}")]
    DuplicateCredentialBinding { handle: ExtensionCredentialHandle },
    #[error("conflicting manifest references for extension {extension_id}")]
    ConflictingManifestReference { extension_id: ExtensionId },
    #[error("conflicting credential bindings for extension {extension_id} and handle {handle}")]
    ConflictingCredentialBinding {
        extension_id: ExtensionId,
        handle: ExtensionCredentialHandle,
    },
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

fn validate_cleanup_id(
    value: String,
    label: &'static str,
) -> Result<String, ExtensionInstallationError> {
    let valid = !value.is_empty()
        && value.len() <= 128
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"._-".contains(&byte)
        })
        && value
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
        && value
            .as_bytes()
            .last()
            .is_some_and(u8::is_ascii_alphanumeric);
    if valid {
        Ok(value)
    } else {
        Err(ExtensionInstallationError::InvalidValue {
            field: label,
            reason: "must be a bounded lowercase identifier".to_string(),
        })
    }
}
