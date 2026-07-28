// arch-exempt: large_file, the three-state lifecycle collapse remains with the installation aggregate pending its planned split, plan #6175
use std::collections::{BTreeSet, HashMap};
use std::fmt;
use std::sync::Arc;

// arch-exempt: large_file, mechanical store-port rename churn only, plan #6263

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ironclaw_filesystem::{
    CasApply, CasExpectation, CasUpdateError, Entry, FilesystemError, Filter, IndexKey, IndexKind,
    IndexName, IndexSpec, IndexValue, Page, RecordKind, RecordVersion, RootFilesystem,
    ScopedFilesystem, VersionedEntry, cas_update,
};
use ironclaw_host_api::{
    ExtensionId, HostPortCatalog, MountAlias, MountGrant, MountPermissions, MountView,
    ResourceScope, ScopedPath, SecretHandle, UserId, VirtualPath, sha256_digest_token,
};
use serde::{Deserialize, Deserializer, Serialize};
use thiserror::Error;

use crate::resolved::ResolvedExtensionManifest;
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
/// Compiled once per install/upgrade: the raw source is kept for diagnostics
/// and recompilation only; production projection reads the
/// [`ResolvedExtensionManifest`] (checklist REC-1). v2 records may still be
/// reprojected from `raw_toml` by domain crates until their cutover phases
/// land.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionManifestRecord {
    raw_toml: String,
    manifest: ExtensionManifestV2,
    resolved: ResolvedExtensionManifest,
    manifest_hash: Option<ManifestHash>,
    removal_cleanup_requirements: Vec<ExtensionRemovalCleanupRequirement>,
}

/// Minimal probe used to dispatch the single parse entry point on the
/// declared schema version (checklist MAN-2).
#[derive(Deserialize)]
struct SchemaVersionProbe {
    #[serde(default)]
    schema_version: String,
}

impl ExtensionManifestRecord {
    /// The single manifest parse entry point: dispatches on the declared
    /// `schema_version` (v2 or v3) and normalizes both into the same
    /// resolved model.
    pub fn from_toml(
        raw_toml: impl Into<String>,
        source: ManifestSource,
        host_port_catalog: &HostPortCatalog,
        manifest_hash: Option<ManifestHash>,
        contracts: &HostApiContractRegistry,
    ) -> Result<Self, ExtensionInstallationError> {
        let raw_toml = raw_toml.into();
        let probe: SchemaVersionProbe = toml::from_str(&raw_toml).map_err(|error| {
            ExtensionInstallationError::InvalidManifest {
                reason: format!("failed to parse extension manifest: {error}"),
            }
        })?;
        let (manifest, resolved) = if probe.schema_version == crate::v3::MANIFEST_SCHEMA_VERSION_V3
        {
            crate::v3::parse_v3(&raw_toml, source, host_port_catalog).map_err(|error| {
                ExtensionInstallationError::InvalidManifest {
                    reason: error.to_string(),
                }
            })?
        } else {
            let manifest =
                ExtensionManifestV2::parse(&raw_toml, source, host_port_catalog, contracts)?;
            let resolved = ResolvedExtensionManifest::from_v2(&manifest);
            (manifest, resolved)
        };
        Ok(Self {
            raw_toml,
            manifest,
            resolved,
            manifest_hash,
            removal_cleanup_requirements: Vec::new(),
        })
    }

    /// Rebuild a record from its persisted resolved contract — no TOML
    /// reparse; the raw source is carried for diagnostics only (checklist
    /// REC-2).
    pub fn from_resolved(
        raw_toml: impl Into<String>,
        source: ManifestSource,
        resolved: ResolvedExtensionManifest,
        manifest_hash: Option<ManifestHash>,
    ) -> Result<Self, ExtensionInstallationError> {
        let manifest = resolved.to_internal(source)?;
        Ok(Self {
            raw_toml: raw_toml.into(),
            manifest,
            resolved,
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

    /// The persisted resolved contract this record was compiled into.
    pub fn resolved(&self) -> &ResolvedExtensionManifest {
        &self.resolved
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

/// The caller-membership axis of an installation.
///
/// `Users` is the only owner shape created by current lifecycle operations:
/// admins and ordinary users install/remove personal membership identically.
/// Tenant-scoped deployment configuration is deliberately not represented by
/// this enum. A future explicit required-extension policy must compose with
/// caller membership rather than overloading owner identity.
///
/// `Tenant` is retained only because persisted records that predate caller
/// ownership omit this field and deserialize through `#[serde(default)]`.
/// Composition narrows those compatibility rows to the configured operator at
/// restore before ordinary lifecycle operations. New code must not create a
/// `Tenant` row or interpret an admin's personal install as tenant-wide.
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

    /// Return the caller-membership rewrite needed to join `user_id`.
    ///
    /// `None` means the user is already a member and the operation is an
    /// idempotent no-op. A legacy `Tenant` compatibility row narrows to the
    /// first explicit caller; current lifecycle code never creates `Tenant`.
    pub fn joined_by(&self, user_id: &UserId) -> Result<Option<Self>, ExtensionInstallationError> {
        match self {
            Self::Tenant => Ok(Some(Self::user(user_id.clone()))),
            Self::Users { user_ids } if user_ids.contains(user_id) => Ok(None),
            Self::Users { user_ids } => {
                let mut joined = user_ids.clone();
                joined.insert(user_id.clone());
                Self::users(joined).map(Some)
            }
        }
    }

    /// Return the remaining caller-membership set after `user_id` leaves.
    /// `None` means no members remain and the shared runtime row may be torn
    /// down. Callers must authorize membership before invoking this method.
    /// A legacy tenant row must first be canonicalized to an explicit member;
    /// this transition never guesses which caller owns that shared row.
    pub fn without_member(
        &self,
        user_id: &UserId,
    ) -> Result<Option<Self>, ExtensionInstallationError> {
        match self {
            Self::Tenant => Err(ExtensionInstallationError::LegacyTenantOwnerNotCanonicalized),
            Self::Users { user_ids } => {
                let remaining = user_ids
                    .iter()
                    .filter(|member| *member != user_id)
                    .cloned()
                    .collect::<BTreeSet<_>>();
                if remaining.is_empty() {
                    Ok(None)
                } else {
                    Self::users(remaining).map(Some)
                }
            }
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
    manifest_ref: ExtensionManifestRef,
    credential_bindings: Vec<ExtensionCredentialBinding>,
    updated_at: DateTime<Utc>,
    // `Tenant` is a read-only compatibility shape for records written before
    // caller-scoped installations. Restore narrows it to the configured
    // operator; new lifecycle operations never create it.
    #[serde(default, skip_serializing_if = "InstallationOwner::is_tenant")]
    owner: InstallationOwner,
}

/// All persisted fields needed to reconstruct an installation without
/// inventing fresh timestamp state.
#[derive(Debug)]
pub struct ExtensionInstallationPersistedParts {
    pub installation_id: ExtensionInstallationId,
    pub extension_id: ExtensionId,
    pub manifest_ref: ExtensionManifestRef,
    pub credential_bindings: Vec<ExtensionCredentialBinding>,
    pub updated_at: DateTime<Utc>,
    pub owner: InstallationOwner,
}

impl ExtensionInstallation {
    pub fn new(
        installation_id: ExtensionInstallationId,
        extension_id: ExtensionId,
        manifest_ref: ExtensionManifestRef,
        credential_bindings: Vec<ExtensionCredentialBinding>,
        updated_at: DateTime<Utc>,
        owner: InstallationOwner,
    ) -> Result<Self, ExtensionInstallationError> {
        Self::from_persisted_parts(ExtensionInstallationPersistedParts {
            installation_id,
            extension_id,
            manifest_ref,
            credential_bindings,
            updated_at,
            owner,
        })
    }

    /// Reconstruct an installation with all state read from persistence.
    ///
    /// Persistence adapters use this neutral constructor when they need to
    /// preserve an existing timestamp while changing the canonical
    /// installation identity.
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
            manifest_ref: parts.manifest_ref,
            credential_bindings: parts.credential_bindings,
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

    pub fn manifest_ref(&self) -> &ExtensionManifestRef {
        &self.manifest_ref
    }

    pub fn credential_bindings(&self) -> &[ExtensionCredentialBinding] {
        &self.credential_bindings
    }

    pub fn updated_at(&self) -> DateTime<Utc> {
        self.updated_at
    }

    pub fn owner(&self) -> &InstallationOwner {
        &self.owner
    }

    /// Same installation with a replaced caller-membership set; refreshes
    /// `updated_at` like every other row mutation.
    pub fn with_owner(mut self, owner: InstallationOwner) -> Self {
        self.owner = owner;
        self.updated_at = Utc::now();
        self
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
            manifest_ref: ExtensionManifestRef,
            credential_bindings: Vec<ExtensionCredentialBinding>,
            // The released aggregate row carries a diagnostic `health`
            // object. It is not modelled here -- the host's activation record
            // already owns extension failure state -- so it is accepted and
            // discarded rather than failing `deny_unknown_fields` on a row
            // written by the previous release.
            #[serde(default)]
            health: serde::de::IgnoredAny,
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
        let _ = wire.health;
        Ok(Self {
            installation_id: wire.installation_id,
            extension_id: wire.extension_id,
            manifest_ref: wire.manifest_ref,
            credential_bindings: wire.credential_bindings,
            updated_at: wire.updated_at,
            owner: wire.owner,
        })
    }
}

/// Generic extension installation state store.
///
/// Implementations own product-agnostic manifest records, installation
/// activation state, opaque credential bindings, and
/// manifest-hash consistency. Domain crates validate domain-specific binding
/// semantics when projecting their host-api sections from these records.
#[async_trait]
pub trait ExtensionInstallationStorePort: Send + Sync {
    async fn list_manifests(
        &self,
    ) -> Result<Vec<ExtensionManifestRecord>, ExtensionInstallationError>;

    async fn get_manifest(
        &self,
        extension_id: &ExtensionId,
    ) -> Result<Option<ExtensionManifestRecord>, ExtensionInstallationError>;

    /// Durably retain `manifest` as removal-cleanup metadata for an extension
    /// with no live installation, so an interrupted orphan cleanup can retry
    /// without the catalog. The definition stays visible to `get_manifest`
    /// until `delete_manifest` marks the removal converged.
    async fn persist_removal_tombstone(
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

    async fn get_installation(
        &self,
        installation_id: &ExtensionInstallationId,
    ) -> Result<Option<ExtensionInstallation>, ExtensionInstallationError>;

    async fn upsert_installation(
        &self,
        installation: ExtensionInstallation,
    ) -> Result<(), ExtensionInstallationError>;

    async fn activate_membership(
        &self,
        installation_id: &ExtensionInstallationId,
        user_id: &UserId,
    ) -> Result<ExtensionInstallation, ExtensionInstallationError>;

    async fn deactivate_membership(
        &self,
        installation_id: &ExtensionInstallationId,
        user_id: &UserId,
    ) -> Result<MembershipDeactivation, ExtensionInstallationError>;

    async fn delete_installation(
        &self,
        installation_id: &ExtensionInstallationId,
    ) -> Result<(), ExtensionInstallationError>;

    async fn delete_manifest(
        &self,
        extension_id: &ExtensionId,
    ) -> Result<(), ExtensionInstallationError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MembershipDeactivation {
    MembershipRemoved(Box<ExtensionInstallation>),
    FinalMemberReserved,
}

#[async_trait]
impl<T> ExtensionInstallationStorePort for Arc<T>
where
    T: ExtensionInstallationStorePort + ?Sized,
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

    async fn persist_removal_tombstone(
        &self,
        manifest: ExtensionManifestRecord,
    ) -> Result<(), ExtensionInstallationError> {
        (**self).persist_removal_tombstone(manifest).await
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

    async fn activate_membership(
        &self,
        installation_id: &ExtensionInstallationId,
        user_id: &UserId,
    ) -> Result<ExtensionInstallation, ExtensionInstallationError> {
        (**self).activate_membership(installation_id, user_id).await
    }

    async fn deactivate_membership(
        &self,
        installation_id: &ExtensionInstallationId,
        user_id: &UserId,
    ) -> Result<MembershipDeactivation, ExtensionInstallationError> {
        (**self)
            .deactivate_membership(installation_id, user_id)
            .await
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
}

const DEFAULT_INSTALLATION_STATE_PATH: &str = "/system/extensions/.installations";
const MANIFEST_RECORD_KIND: &str = "extension_manifest_record";
const INSTALLATION_RECORD_KIND: &str = "extension_installation_record";
const EXTENSION_STATE_V2_SCHEMA: &str = "extension_state.v2";
const INSTALLATION_RECORD_KIND_V2: &str = "extension_installation_record_v2";
const MEMBERSHIP_RECORD_KIND_V2: &str = "extension_membership_record_v2";
const CREDENTIAL_BINDING_RECORD_KIND_V2: &str = "extension_credential_binding_record_v2";
const FILESYSTEM_CAS_RETRIES: usize = 5;

/// An exclusive mutation lease on an installation record — the cross-process
/// gate for multi-row transitions. While a lease is held the installation is
/// hidden from typed reads (its manifest stays readable) so readers never see
/// a torn aggregate. `member: None` is a whole-aggregate mutation (update or
/// full removal); `member: Some(user)` is that member's removal, recorded so
/// the same caller's retry re-enters its own lease and no other flow can
/// complete it. There is no expiry: a lease orphaned by a crash is resolved
/// by startup repair (restore from the compatibility snapshot, or finish the
/// removal).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct V2MutationLease {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    member: Option<UserId>,
}

impl V2MutationLease {
    fn update() -> Self {
        Self { member: None }
    }

    fn member_removal(user_id: UserId) -> Self {
        Self {
            member: Some(user_id),
        }
    }
}

/// The installation record: the deployment's install pin (the embedded,
/// hash-carrying manifest wire) plus instance lifecycle. Lifecycle state is
/// derived, not stored: `removed_at: Some` is the tombstone, `lease: Some` is
/// an in-flight exclusive mutation, and a record with neither is live.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct V2InstallationRecord {
    schema_version: String,
    installation_id: ExtensionInstallationId,
    extension_id: ExtensionId,
    manifest: WireManifestRecord,
    legacy_tenant_owner: bool,
    updated_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    removed_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    lease: Option<V2MutationLease>,
    /// A tombstoned record whose removal cleanup has not yet converged. While
    /// set, the embedded definition stays authoritative for `get_manifest`
    /// readers so removal retries work without the catalog and fresh imports
    /// stay blocked until `delete_manifest` marks convergence. Fails open to
    /// `false` only for records written before the field existed, all of
    /// which had already converged.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    removal_cleanup_pending: bool,
}

impl V2InstallationRecord {
    fn is_removed(&self) -> bool {
        self.removed_at.is_some()
    }

    /// Visible = reconstructable by typed reads: not removed, no lease held.
    fn is_visible(&self) -> bool {
        self.removed_at.is_none() && self.lease.is_none()
    }

    /// Whether the embedded definition is authoritative for manifest readers:
    /// every non-removed record, plus removal-cleanup tombstones that have
    /// not yet converged.
    fn manifest_is_authoritative(&self) -> bool {
        !self.is_removed() || self.removal_cleanup_pending
    }

    fn manifest_ref(&self) -> ExtensionManifestRef {
        ExtensionManifestRef::new(
            self.extension_id.clone(),
            self.manifest.manifest_hash.clone(),
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct V2MembershipRecord {
    schema_version: String,
    installation_id: ExtensionInstallationId,
    user_id: UserId,
    installed_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    removed_at: Option<DateTime<Utc>>,
}

impl V2MembershipRecord {
    fn is_active(&self) -> bool {
        self.removed_at.is_none()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct V2CredentialBindingRecord {
    schema_version: String,
    installation_id: ExtensionInstallationId,
    credential_handle: ExtensionCredentialHandle,
    secret_handle: SecretHandle,
    position: u32,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    removed_at: Option<DateTime<Utc>>,
}

impl V2CredentialBindingRecord {
    fn is_active(&self) -> bool {
        self.removed_at.is_none()
    }
}

/// Filesystem-backed extension installation state store.
///
/// Manifests, installation identity, user memberships, and credential
/// bindings are persisted as separate typed record rows under the configured
/// root path. Secondary indexes are declared on the row prefixes so scans that
/// gate lifecycle behavior can use the filesystem query API instead of loading
/// or rewriting a monolithic state snapshot.
pub struct ExtensionInstallationStore {
    filesystem: Arc<dyn RootFilesystem>,
    scoped_filesystem: Arc<ScopedFilesystem<dyn RootFilesystem>>,
    root: VirtualPath,
    host_ports: HostPortCatalog,
    contracts: HostApiContractRegistry,
    cas_retries: usize,
}

impl fmt::Debug for ExtensionInstallationStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExtensionInstallationStore")
            .field("root", &self.root)
            .field("cas_retries", &self.cas_retries)
            .finish_non_exhaustive()
    }
}

impl ExtensionInstallationStore {
    pub async fn load_at(
        filesystem: Arc<dyn RootFilesystem>,
        root: VirtualPath,
        host_ports: HostPortCatalog,
        contracts: HostApiContractRegistry,
    ) -> Result<Self, ExtensionInstallationError> {
        let mount_view = MountView::new(vec![MountGrant::new(
            MountAlias::new("/extension-state").map_err(invalid_installation_error)?,
            root.clone(),
            MountPermissions::read_write_list_delete(),
        )])
        .map_err(invalid_installation_error)?;
        let scoped_filesystem = Arc::new(ScopedFilesystem::with_fixed_view(
            Arc::clone(&filesystem),
            mount_view,
        ));
        let store = Self {
            filesystem,
            scoped_filesystem,
            root,
            host_ports,
            contracts,
            cas_retries: FILESYSTEM_CAS_RETRIES,
        };
        store.ensure_indexes().await?;
        store.migrate_legacy_manifest_rows().await?;
        store.ensure_v2_indexes().await?;
        store.bootstrap_v2_from_compatibility_rows().await?;
        store.repair_interrupted_v2_leases().await?;
        store.repair_removed_v2_children().await?;
        store.repair_compatibility_views().await?;
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
        .await
    }

    async fn ensure_v2_indexes(&self) -> Result<(), ExtensionInstallationError> {
        for (prefix, name, key) in [
            (
                self.v2_installations_root()?,
                "extension_v2_installations_by_installation_id",
                "installation_id",
            ),
            (
                self.v2_installations_root()?,
                "extension_v2_installations_by_extension_id",
                "extension_id",
            ),
            (
                self.v2_memberships_root()?,
                "extension_v2_memberships_by_installation_id",
                "installation_id",
            ),
            (
                self.v2_memberships_root()?,
                "extension_v2_memberships_by_user_id",
                "user_id",
            ),
            (
                self.v2_credential_bindings_root()?,
                "extension_v2_bindings_by_installation_id",
                "installation_id",
            ),
        ] {
            self.ensure_exact_index(&prefix, name, key).await?;
        }
        Ok(())
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

    /// One-time compatibility compiler for rows written by the filesystem
    /// store before resolved manifests became authoritative. The migrated row
    /// is rewritten under CAS before `load_at` returns, so normal projection
    /// paths never reparse raw TOML.
    async fn migrate_legacy_manifest_rows(&self) -> Result<(), ExtensionInstallationError> {
        let rows = query_all(&self.filesystem, &self.manifests_root()?, &Filter::All).await?;
        for row in rows {
            let mut complete = false;
            for _ in 0..=self.cas_retries {
                let Some(current) = self
                    .filesystem
                    .get(&row.path)
                    .await
                    .map_err(store_unavailable("load legacy extension manifest row"))?
                else {
                    complete = true;
                    break;
                };
                ensure_entry_kind(&current.entry, MANIFEST_RECORD_KIND, &row.path)?;
                let wire: WireManifestRecord = current.entry.parse_json().map_err(|error| {
                    corrupt_row(
                        "deserialize legacy extension manifest row",
                        &row.path,
                        error,
                    )
                })?;
                if wire.resolved.is_some() {
                    complete = true;
                    break;
                }
                let record = ExtensionManifestRecord::from_toml(
                    wire.raw_toml,
                    wire.source.into_manifest_source(),
                    &self.host_ports,
                    wire.manifest_hash,
                    &self.contracts,
                )?
                .with_removal_cleanup_requirements(wire.removal_cleanup_requirements);
                match self
                    .filesystem
                    .put(
                        &row.path,
                        entry_for_manifest(&record)?,
                        CasExpectation::Version(current.version),
                    )
                    .await
                {
                    Ok(_) => {
                        complete = true;
                        break;
                    }
                    Err(FilesystemError::VersionMismatch { .. }) => continue,
                    Err(error) => {
                        return Err(store_unavailable("migrate extension manifest row")(error));
                    }
                }
            }
            if !complete {
                return Err(store_unavailable_error(
                    "legacy extension manifest row changed repeatedly while migrating",
                ));
            }
        }
        Ok(())
    }

    fn manifests_root(&self) -> Result<VirtualPath, ExtensionInstallationError> {
        child_path(&self.root, "manifests")
    }

    fn installations_root(&self) -> Result<VirtualPath, ExtensionInstallationError> {
        child_path(&self.root, "installations")
    }

    fn v2_root(&self) -> Result<VirtualPath, ExtensionInstallationError> {
        child_path(&self.root, "v2")
    }

    fn v2_installations_root(&self) -> Result<VirtualPath, ExtensionInstallationError> {
        child_path(&self.v2_root()?, "installations")
    }

    fn v2_memberships_root(&self) -> Result<VirtualPath, ExtensionInstallationError> {
        child_path(&self.v2_root()?, "memberships")
    }

    fn v2_credential_bindings_root(&self) -> Result<VirtualPath, ExtensionInstallationError> {
        child_path(&self.v2_root()?, "credential-bindings")
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

    fn v2_installation_path(
        &self,
        installation_id: &ExtensionInstallationId,
    ) -> Result<VirtualPath, ExtensionInstallationError> {
        child_path(
            &self.v2_installations_root()?,
            &format!("{}.json", row_token(installation_id.as_str())),
        )
    }

    fn v2_membership_root(
        &self,
        installation_id: &ExtensionInstallationId,
    ) -> Result<VirtualPath, ExtensionInstallationError> {
        child_path(
            &self.v2_memberships_root()?,
            &row_token(installation_id.as_str()),
        )
    }

    fn v2_membership_path(
        &self,
        installation_id: &ExtensionInstallationId,
        user_id: &UserId,
    ) -> Result<VirtualPath, ExtensionInstallationError> {
        child_path(
            &self.v2_membership_root(installation_id)?,
            &format!("{}.json", row_token(user_id.as_str())),
        )
    }

    fn v2_membership_scoped_path(
        &self,
        installation_id: &ExtensionInstallationId,
        user_id: &UserId,
    ) -> Result<ScopedPath, ExtensionInstallationError> {
        self.v2_scoped_path(&self.v2_membership_path(installation_id, user_id)?)
    }

    fn v2_scoped_path(
        &self,
        virtual_path: &VirtualPath,
    ) -> Result<ScopedPath, ExtensionInstallationError> {
        let relative = virtual_path
            .as_str()
            .strip_prefix(self.root.as_str())
            .ok_or_else(|| {
                invalid_installation_error("v2 extension state path escaped the installation root")
            })?;
        ScopedPath::new(format!("/extension-state{relative}")).map_err(invalid_installation_error)
    }

    fn v2_credential_binding_root(
        &self,
        installation_id: &ExtensionInstallationId,
    ) -> Result<VirtualPath, ExtensionInstallationError> {
        child_path(
            &self.v2_credential_bindings_root()?,
            &row_token(installation_id.as_str()),
        )
    }

    fn v2_credential_binding_path(
        &self,
        installation_id: &ExtensionInstallationId,
        credential_handle: &ExtensionCredentialHandle,
    ) -> Result<VirtualPath, ExtensionInstallationError> {
        child_path(
            &self.v2_credential_binding_root(installation_id)?,
            &format!("{}.json", row_token(credential_handle.as_str())),
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

    async fn query_installations(
        &self,
        filter: &Filter,
    ) -> Result<Vec<ExtensionInstallation>, ExtensionInstallationError> {
        let rows = query_all(&self.filesystem, &self.installations_root()?, filter).await?;
        rows.into_iter()
            .map(|entry| parse_installation_entry(entry.entry, &entry.path))
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
        wire.into_manifest_record()
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

    /// All merged records for one extension id. Installation ids equal
    /// package ids today so this is normally zero or one record, but legacy
    /// data may hold duplicates, so extension-level decisions must consider
    /// every record.
    async fn load_v2_records_by_extension(
        &self,
        extension_id: &ExtensionId,
    ) -> Result<Vec<V2InstallationRecord>, ExtensionInstallationError> {
        let rows = query_all(
            &self.filesystem,
            &self.v2_installations_root()?,
            &Filter::Eq {
                key: index_key("extension_id")?,
                value: IndexValue::Text(extension_id.as_str().to_string()),
            },
        )
        .await?;
        let mut records = rows
            .into_iter()
            .map(|row| parse_v2_installation_entry(row.entry, &row.path))
            .collect::<Result<Vec<_>, _>>()?;
        records.retain(|record| &record.extension_id == extension_id);
        // Live records first so extension-level reads never answer from a
        // tombstone while a sibling record is still live.
        records.sort_by(|a, b| {
            a.is_removed()
                .cmp(&b.is_removed())
                .then_with(|| a.installation_id.cmp(&b.installation_id))
        });
        Ok(records)
    }

    async fn load_v2_installation_record(
        &self,
        installation_id: &ExtensionInstallationId,
    ) -> Result<Option<(V2InstallationRecord, RecordVersion)>, ExtensionInstallationError> {
        let path = self.v2_installation_path(installation_id)?;
        let Some(entry) = self
            .filesystem
            .get(&path)
            .await
            .map_err(store_unavailable("load v2 extension installation row"))?
        else {
            return Ok(None);
        };
        let record = parse_v2_installation_entry(entry.entry, &path)?;
        if &record.installation_id != installation_id {
            return Err(invalid_installation_error(format!(
                "v2 extension installation row key {installation_id} contained installation {}",
                record.installation_id
            )));
        }
        Ok(Some((record, entry.version)))
    }

    /// Write the full aggregate. `manifest: Some` re-pins the embedded
    /// definition (install/update); `None` carries the existing row's
    /// definition forward (reactivation, restore, plain aggregate rewrites).
    async fn put_v2_installation(
        &self,
        installation: &ExtensionInstallation,
        manifest: Option<&ExtensionManifestRecord>,
    ) -> Result<(), ExtensionInstallationError> {
        let prior = match self
            .load_v2_installation_record(installation.installation_id())
            .await?
        {
            Some((core, _)) if core.is_visible() => {
                let prior = self.reconstruct_v2_installation(&core).await?;
                self.take_v2_update_lease(installation.installation_id())
                    .await?;
                Some(prior)
            }
            _ => None,
        };
        // Every aggregate write sweeps child rows omitted from its member
        // and binding sets. Under the single-writer-per-root deployment
        // contract an omitted-but-active row can only be debris from an
        // earlier failed write (an interrupted install's membership row, a
        // crashed update's leftovers) — merging it in would grant membership
        // to a user whose install never succeeded.
        if let Err(error) = self
            .write_v2_installation_components(installation, manifest)
            .await
        {
            if let Some(prior) = prior
                && self
                    .write_v2_installation_components(&prior, None)
                    .await
                    .is_err()
            {
                return Err(store_unavailable_error(
                    "aggregate update failed and its prior installation could not be restored",
                ));
            }
            return Err(error);
        }
        Ok(())
    }

    async fn write_v2_installation_components(
        &self,
        installation: &ExtensionInstallation,
        manifest: Option<&ExtensionManifestRecord>,
    ) -> Result<(), ExtensionInstallationError> {
        let desired_members = installation.owner().members().cloned().unwrap_or_default();
        for user_id in &desired_members {
            self.activate_v2_membership_row(
                installation.installation_id(),
                user_id,
                installation.updated_at(),
            )
            .await?;
        }
        let existing_memberships = self
            .query_v2_memberships(installation.installation_id())
            .await?;
        for membership in existing_memberships {
            if desired_members.contains(&membership.user_id) {
                continue;
            }
            if membership.is_active() {
                self.deactivate_v2_membership_row(
                    installation.installation_id(),
                    &membership.user_id,
                    installation.updated_at(),
                )
                .await?;
            }
        }

        let desired_binding_handles = installation
            .credential_bindings()
            .iter()
            .map(|binding| binding.credential_handle().clone())
            .collect::<BTreeSet<_>>();
        for (position, binding) in installation.credential_bindings().iter().enumerate() {
            let position = u32::try_from(position).map_err(|_| {
                invalid_installation_error("extension credential binding position overflow")
            })?;
            self.activate_v2_credential_binding_row(
                installation.installation_id(),
                binding,
                position,
                installation.updated_at(),
            )
            .await?;
        }
        let existing_bindings = self
            .query_v2_credential_bindings(installation.installation_id())
            .await?;
        for binding in existing_bindings {
            if desired_binding_handles.contains(&binding.credential_handle) {
                continue;
            }
            if binding.is_active() {
                self.deactivate_v2_credential_binding_row(
                    installation.installation_id(),
                    &binding.credential_handle,
                    installation.updated_at(),
                )
                .await?;
            }
        }

        self.put_v2_installation_core(installation, manifest)
            .await
            .map(|_| ())
    }

    async fn take_v2_update_lease(
        &self,
        installation_id: &ExtensionInstallationId,
    ) -> Result<(), ExtensionInstallationError> {
        let path = self.v2_scoped_path(&self.v2_installation_path(installation_id)?)?;
        let installation_id = installation_id.clone();
        cas_update(
            self.scoped_filesystem.as_ref(),
            &ResourceScope::system(),
            &path,
            decode_v2_installation_body,
            entry_for_v2_installation,
            move |current: Option<V2InstallationRecord>| {
                let installation_id = installation_id.clone();
                async move {
                    let Some(mut record) = current else {
                        return Err(ExtensionInstallationError::InstallationNotFound {
                            installation_id,
                        });
                    };
                    if record.installation_id != installation_id {
                        return Err(invalid_installation_error(
                            "v2 installation body identity did not match its key",
                        ));
                    }
                    if !record.is_visible() {
                        return Err(ExtensionInstallationError::MembershipMutationInProgress {
                            installation_id,
                        });
                    }
                    record.lease = Some(V2MutationLease::update());
                    record.updated_at = Utc::now();
                    Ok(CasApply::new(record, ()))
                }
            },
        )
        .await
        .map_err(|error| map_extension_state_cas_error(error, "installation"))
    }

    async fn put_v2_installation_core(
        &self,
        installation: &ExtensionInstallation,
        manifest: Option<&ExtensionManifestRecord>,
    ) -> Result<V2InstallationRecord, ExtensionInstallationError> {
        let path =
            self.v2_scoped_path(&self.v2_installation_path(installation.installation_id())?)?;
        let installation = installation.clone();
        let provided_wire = manifest.map(WireManifestRecord::from);
        cas_update(
            self.scoped_filesystem.as_ref(),
            &ResourceScope::system(),
            &path,
            decode_v2_installation_body,
            entry_for_v2_installation,
            move |current: Option<V2InstallationRecord>| {
                let installation = installation.clone();
                let provided_wire = provided_wire.clone();
                async move {
                    if current.as_ref().is_some_and(|current| {
                        current.installation_id != *installation.installation_id()
                    }) {
                        return Err(invalid_installation_error(
                            "v2 installation body identity did not match its key",
                        ));
                    }
                    // The commit either re-pins a provided definition or
                    // carries the existing row's forward; a fresh id with no
                    // definition to embed cannot become installed.
                    let Some(wire) = provided_wire
                        .or_else(|| current.as_ref().map(|record| record.manifest.clone()))
                    else {
                        return Err(ExtensionInstallationError::UnknownManifest {
                            extension_id: installation.extension_id().clone(),
                        });
                    };
                    let record = V2InstallationRecord {
                        schema_version: EXTENSION_STATE_V2_SCHEMA.to_string(),
                        installation_id: installation.installation_id().clone(),
                        extension_id: installation.extension_id().clone(),
                        manifest: wire,
                        legacy_tenant_owner: installation.owner().is_tenant(),
                        updated_at: installation.updated_at(),
                        removed_at: None,
                        lease: None,
                        removal_cleanup_pending: false,
                    };
                    Ok(CasApply::new(record.clone(), record))
                }
            },
        )
        .await
        .map_err(|error| map_extension_state_cas_error(error, "installation"))
    }

    async fn canonicalize_v2_installation_owner(
        &self,
        installation_id: &ExtensionInstallationId,
        now: DateTime<Utc>,
    ) -> Result<V2InstallationRecord, ExtensionInstallationError> {
        let path = self.v2_scoped_path(&self.v2_installation_path(installation_id)?)?;
        let installation_id = installation_id.clone();
        cas_update(
            self.scoped_filesystem.as_ref(),
            &ResourceScope::system(),
            &path,
            decode_v2_installation_body,
            entry_for_v2_installation,
            move |current: Option<V2InstallationRecord>| {
                let installation_id = installation_id.clone();
                async move {
                    let Some(mut record) = current else {
                        return Err(ExtensionInstallationError::InstallationNotFound {
                            installation_id,
                        });
                    };
                    if record.installation_id != installation_id {
                        return Err(invalid_installation_error(
                            "v2 installation body identity did not match its key",
                        ));
                    }
                    if record.is_removed() {
                        return Err(ExtensionInstallationError::InstallationNotFound {
                            installation_id,
                        });
                    }
                    if record.lease.is_some() {
                        return Err(ExtensionInstallationError::MembershipMutationInProgress {
                            installation_id,
                        });
                    }
                    if record.legacy_tenant_owner {
                        record.legacy_tenant_owner = false;
                        record.updated_at = now;
                    }
                    Ok(CasApply::new(record.clone(), record))
                }
            },
        )
        .await
        .map_err(|error| map_extension_state_cas_error(error, "installation"))
    }

    async fn reserve_v2_membership_removal(
        &self,
        installation_id: &ExtensionInstallationId,
        user_id: &UserId,
        now: DateTime<Utc>,
    ) -> Result<V2InstallationRecord, ExtensionInstallationError> {
        let path = self.v2_scoped_path(&self.v2_installation_path(installation_id)?)?;
        let installation_id = installation_id.clone();
        let user_id = user_id.clone();
        cas_update(
            self.scoped_filesystem.as_ref(),
            &ResourceScope::system(),
            &path,
            decode_v2_installation_body,
            entry_for_v2_installation,
            move |current: Option<V2InstallationRecord>| {
                let installation_id = installation_id.clone();
                let user_id = user_id.clone();
                async move {
                    let Some(mut record) = current else {
                        return Err(ExtensionInstallationError::InstallationNotFound {
                            installation_id,
                        });
                    };
                    if record.installation_id != installation_id {
                        return Err(invalid_installation_error(
                            "v2 installation body identity did not match its key",
                        ));
                    }
                    if record.is_removed() {
                        return Err(ExtensionInstallationError::InstallationNotFound {
                            installation_id,
                        });
                    }
                    if let Some(lease) = record.lease.as_ref() {
                        if lease.member.as_ref() == Some(&user_id) {
                            return Ok(CasApply::new(record.clone(), record));
                        }
                        return Err(ExtensionInstallationError::MembershipMutationInProgress {
                            installation_id,
                        });
                    }
                    record.lease = Some(V2MutationLease::member_removal(user_id));
                    record.updated_at = now;
                    Ok(CasApply::new(record.clone(), record))
                }
            },
        )
        .await
        .map_err(|error| map_extension_state_cas_error(error, "installation"))
    }

    async fn begin_v2_installation_removal(
        &self,
        installation_id: &ExtensionInstallationId,
        now: DateTime<Utc>,
    ) -> Result<(), ExtensionInstallationError> {
        let path = self.v2_scoped_path(&self.v2_installation_path(installation_id)?)?;
        let installation_id = installation_id.clone();
        cas_update(
            self.scoped_filesystem.as_ref(),
            &ResourceScope::system(),
            &path,
            decode_v2_installation_body,
            entry_for_v2_installation,
            move |current: Option<V2InstallationRecord>| {
                let installation_id = installation_id.clone();
                async move {
                    let Some(mut record) = current else {
                        return Err(ExtensionInstallationError::InstallationNotFound {
                            installation_id,
                        });
                    };
                    if record.installation_id != installation_id {
                        return Err(invalid_installation_error(
                            "v2 installation body identity did not match its key",
                        ));
                    }
                    // A visible record takes the exclusive lease; an existing
                    // lease (this removal's own retry, or the member-removal
                    // lease this final removal continues) and an existing
                    // tombstone both pass through so removal stays idempotent.
                    if record.is_visible() {
                        record.lease = Some(V2MutationLease::update());
                        record.updated_at = now;
                    }
                    Ok(CasApply::new(record, ()))
                }
            },
        )
        .await
        .map_err(|error| map_extension_state_cas_error(error, "installation"))
    }

    async fn finish_v2_membership_removal(
        &self,
        installation_id: &ExtensionInstallationId,
        user_id: &UserId,
        now: DateTime<Utc>,
    ) -> Result<V2InstallationRecord, ExtensionInstallationError> {
        let path = self.v2_scoped_path(&self.v2_installation_path(installation_id)?)?;
        let installation_id = installation_id.clone();
        let user_id = user_id.clone();
        cas_update(
            self.scoped_filesystem.as_ref(),
            &ResourceScope::system(),
            &path,
            decode_v2_installation_body,
            entry_for_v2_installation,
            move |current: Option<V2InstallationRecord>| {
                let installation_id = installation_id.clone();
                let user_id = user_id.clone();
                async move {
                    let Some(mut record) = current else {
                        return Err(ExtensionInstallationError::InstallationNotFound {
                            installation_id,
                        });
                    };
                    if record.installation_id != installation_id {
                        return Err(invalid_installation_error(
                            "v2 installation body identity did not match its key",
                        ));
                    }
                    if record.is_removed()
                        || record
                            .lease
                            .as_ref()
                            .is_none_or(|lease| lease.member.as_ref() != Some(&user_id))
                    {
                        return Err(ExtensionInstallationError::MembershipMutationInProgress {
                            installation_id,
                        });
                    }
                    record.lease = None;
                    record.updated_at = now;
                    Ok(CasApply::new(record.clone(), record))
                }
            },
        )
        .await
        .map_err(|error| map_extension_state_cas_error(error, "installation"))
    }

    async fn finish_v2_installation_removal(
        &self,
        installation_id: &ExtensionInstallationId,
        now: DateTime<Utc>,
    ) -> Result<(), ExtensionInstallationError> {
        let path = self.v2_scoped_path(&self.v2_installation_path(installation_id)?)?;
        let installation_id = installation_id.clone();
        cas_update(
            self.scoped_filesystem.as_ref(),
            &ResourceScope::system(),
            &path,
            decode_v2_installation_body,
            entry_for_v2_installation,
            move |current: Option<V2InstallationRecord>| {
                let installation_id = installation_id.clone();
                async move {
                    let Some(mut record) = current else {
                        return Err(ExtensionInstallationError::InstallationNotFound {
                            installation_id,
                        });
                    };
                    if record.installation_id != installation_id {
                        return Err(invalid_installation_error(
                            "v2 installation body identity did not match its key",
                        ));
                    }
                    if !record.is_removed() {
                        record.removed_at = Some(now);
                        record.updated_at = now;
                        // The embedded definition stays authoritative for
                        // cleanup retries until `delete_manifest` marks the
                        // removal converged.
                        record.removal_cleanup_pending = true;
                    }
                    record.lease = None;
                    Ok(CasApply::new(record, ()))
                }
            },
        )
        .await
        .map_err(|error| map_extension_state_cas_error(error, "installation"))
    }

    /// Clear the removal-cleanup-pending marker on a tombstoned record:
    /// `delete_manifest` calls this when removal cleanup has converged, after
    /// which the embedded definition stops being authoritative for manifest
    /// readers and fresh imports are unblocked.
    async fn mark_v2_removal_converged(
        &self,
        installation_id: &ExtensionInstallationId,
    ) -> Result<(), ExtensionInstallationError> {
        let path = self.v2_scoped_path(&self.v2_installation_path(installation_id)?)?;
        let installation_id = installation_id.clone();
        cas_update(
            self.scoped_filesystem.as_ref(),
            &ResourceScope::system(),
            &path,
            decode_v2_installation_body,
            entry_for_v2_installation,
            move |current: Option<V2InstallationRecord>| {
                let installation_id = installation_id.clone();
                async move {
                    let Some(mut record) = current else {
                        return Err(ExtensionInstallationError::InstallationNotFound {
                            installation_id,
                        });
                    };
                    if record.installation_id != installation_id {
                        return Err(invalid_installation_error(
                            "v2 installation body identity did not match its key",
                        ));
                    }
                    if record.is_removed() && record.removal_cleanup_pending {
                        record.removal_cleanup_pending = false;
                        record.updated_at = Utc::now();
                    }
                    Ok(CasApply::new(record, ()))
                }
            },
        )
        .await
        .map_err(|error| map_extension_state_cas_error(error, "installation"))
    }

    async fn activate_v2_membership_row(
        &self,
        installation_id: &ExtensionInstallationId,
        user_id: &UserId,
        now: DateTime<Utc>,
    ) -> Result<bool, ExtensionInstallationError> {
        let path = self.v2_membership_scoped_path(installation_id, user_id)?;
        let installation_id = installation_id.clone();
        let user_id = user_id.clone();
        cas_update(
            self.scoped_filesystem.as_ref(),
            &ResourceScope::system(),
            &path,
            decode_v2_membership_body,
            entry_for_v2_membership,
            move |current: Option<V2MembershipRecord>| {
                let installation_id = installation_id.clone();
                let user_id = user_id.clone();
                async move {
                    if let Some(record) = current.as_ref()
                        && (record.installation_id != installation_id || record.user_id != user_id)
                    {
                        return Err(invalid_installation_error(
                            "v2 membership body identity did not match its key",
                        ));
                    }
                    let already_active =
                        current.as_ref().is_some_and(V2MembershipRecord::is_active);
                    let installed_at = current
                        .as_ref()
                        .map(|record| record.installed_at)
                        .unwrap_or(now);
                    Ok(CasApply::new(
                        V2MembershipRecord {
                            schema_version: EXTENSION_STATE_V2_SCHEMA.to_string(),
                            installation_id,
                            user_id,
                            installed_at,
                            updated_at: if already_active {
                                current
                                    .as_ref()
                                    .map(|record| record.updated_at)
                                    .unwrap_or(now)
                            } else {
                                now
                            },
                            removed_at: None,
                        },
                        !already_active,
                    ))
                }
            },
        )
        .await
        .map_err(|error| map_extension_state_cas_error(error, "membership"))
    }

    async fn deactivate_v2_membership_row(
        &self,
        installation_id: &ExtensionInstallationId,
        user_id: &UserId,
        now: DateTime<Utc>,
    ) -> Result<bool, ExtensionInstallationError> {
        let virtual_path = self.v2_membership_path(installation_id, user_id)?;
        let Some(current) = self
            .filesystem
            .get(&virtual_path)
            .await
            .map_err(store_unavailable("load v2 extension membership row"))?
        else {
            return Ok(false);
        };
        let current = parse_v2_membership_entry(current.entry, &virtual_path)?;
        if !current.is_active() {
            return Ok(false);
        }

        let path = self.v2_membership_scoped_path(installation_id, user_id)?;
        let installation_id = installation_id.clone();
        let user_id = user_id.clone();
        cas_update(
            self.scoped_filesystem.as_ref(),
            &ResourceScope::system(),
            &path,
            decode_v2_membership_body,
            entry_for_v2_membership,
            move |current: Option<V2MembershipRecord>| {
                let installation_id = installation_id.clone();
                let user_id = user_id.clone();
                async move {
                    let Some(mut record) = current else {
                        return Err(invalid_installation_error(
                            "v2 membership disappeared during deactivation",
                        ));
                    };
                    if record.installation_id != installation_id || record.user_id != user_id {
                        return Err(invalid_installation_error(
                            "v2 membership body identity did not match its key",
                        ));
                    }
                    if !record.is_active() {
                        return Ok(CasApply::new(record, false));
                    }
                    record.updated_at = now;
                    record.removed_at = Some(now);
                    Ok(CasApply::new(record, true))
                }
            },
        )
        .await
        .map_err(|error| map_extension_state_cas_error(error, "membership"))
    }

    async fn activate_v2_credential_binding_row(
        &self,
        installation_id: &ExtensionInstallationId,
        binding: &ExtensionCredentialBinding,
        position: u32,
        now: DateTime<Utc>,
    ) -> Result<(), ExtensionInstallationError> {
        let path = self.v2_scoped_path(
            &self.v2_credential_binding_path(installation_id, binding.credential_handle())?,
        )?;
        let installation_id = installation_id.clone();
        let credential_handle = binding.credential_handle().clone();
        let secret_handle = binding.secret_handle().clone();
        cas_update(
            self.scoped_filesystem.as_ref(),
            &ResourceScope::system(),
            &path,
            decode_v2_credential_binding_body,
            entry_for_v2_credential_binding,
            move |current: Option<V2CredentialBindingRecord>| {
                let installation_id = installation_id.clone();
                let credential_handle = credential_handle.clone();
                let secret_handle = secret_handle.clone();
                async move {
                    if current.as_ref().is_some_and(|record| {
                        record.installation_id != installation_id
                            || record.credential_handle != credential_handle
                    }) {
                        return Err(invalid_installation_error(
                            "v2 credential binding body identity did not match its key",
                        ));
                    }
                    Ok(CasApply::new(
                        V2CredentialBindingRecord {
                            schema_version: EXTENSION_STATE_V2_SCHEMA.to_string(),
                            installation_id,
                            credential_handle,
                            secret_handle,
                            position,
                            created_at: current
                                .as_ref()
                                .map(|record| record.created_at)
                                .unwrap_or(now),
                            updated_at: now,
                            removed_at: None,
                        },
                        (),
                    ))
                }
            },
        )
        .await
        .map_err(|error| map_extension_state_cas_error(error, "credential binding"))
    }

    async fn deactivate_v2_credential_binding_row(
        &self,
        installation_id: &ExtensionInstallationId,
        credential_handle: &ExtensionCredentialHandle,
        now: DateTime<Utc>,
    ) -> Result<(), ExtensionInstallationError> {
        let path = self.v2_scoped_path(
            &self.v2_credential_binding_path(installation_id, credential_handle)?,
        )?;
        let installation_id = installation_id.clone();
        let credential_handle = credential_handle.clone();
        cas_update(
            self.scoped_filesystem.as_ref(),
            &ResourceScope::system(),
            &path,
            decode_v2_credential_binding_body,
            entry_for_v2_credential_binding,
            move |current: Option<V2CredentialBindingRecord>| {
                let installation_id = installation_id.clone();
                let credential_handle = credential_handle.clone();
                async move {
                    let Some(mut record) = current else {
                        return Err(invalid_installation_error(
                            "v2 credential binding disappeared during deactivation",
                        ));
                    };
                    if record.installation_id != installation_id
                        || record.credential_handle != credential_handle
                    {
                        return Err(invalid_installation_error(
                            "v2 credential binding body identity did not match its key",
                        ));
                    }
                    if record.is_active() {
                        record.updated_at = now;
                        record.removed_at = Some(now);
                    }
                    Ok(CasApply::new(record, ()))
                }
            },
        )
        .await
        .map_err(|error| map_extension_state_cas_error(error, "credential binding"))
    }

    async fn query_v2_memberships(
        &self,
        installation_id: &ExtensionInstallationId,
    ) -> Result<Vec<V2MembershipRecord>, ExtensionInstallationError> {
        let rows = query_all(
            &self.filesystem,
            &self.v2_membership_root(installation_id)?,
            &Filter::All,
        )
        .await?;
        rows.into_iter()
            .map(|row| {
                let record = parse_v2_membership_entry(row.entry, &row.path)?;
                if &record.installation_id != installation_id {
                    return Err(invalid_installation_error(
                        "v2 membership row installation id did not match its parent",
                    ));
                }
                Ok(record)
            })
            .collect()
    }

    async fn query_v2_credential_bindings(
        &self,
        installation_id: &ExtensionInstallationId,
    ) -> Result<Vec<V2CredentialBindingRecord>, ExtensionInstallationError> {
        let rows = query_all(
            &self.filesystem,
            &self.v2_credential_binding_root(installation_id)?,
            &Filter::All,
        )
        .await?;
        rows.into_iter()
            .map(|row| {
                let record = parse_v2_credential_binding_entry(row.entry, &row.path)?;
                if &record.installation_id != installation_id {
                    return Err(invalid_installation_error(
                        "v2 credential binding installation id did not match its parent",
                    ));
                }
                Ok(record)
            })
            .collect()
    }

    async fn deactivate_reserved_v2_membership(
        &self,
        installation_id: &ExtensionInstallationId,
        user_id: &UserId,
        now: DateTime<Utc>,
    ) -> Result<MembershipDeactivation, ExtensionInstallationError> {
        let active_memberships = self
            .query_v2_memberships(installation_id)
            .await?
            .into_iter()
            .filter(V2MembershipRecord::is_active)
            .collect::<Vec<_>>();
        let caller_is_active = active_memberships
            .iter()
            .any(|record| &record.user_id == user_id);
        let other_active_members = active_memberships
            .iter()
            .any(|record| &record.user_id != user_id);
        if !other_active_members {
            return Ok(MembershipDeactivation::FinalMemberReserved);
        }
        if caller_is_active {
            self.deactivate_v2_membership_row(installation_id, user_id, now)
                .await?;
        }
        let other_active_after_deactivation = self
            .query_v2_memberships(installation_id)
            .await?
            .into_iter()
            .any(|record| record.is_active() && &record.user_id != user_id);
        if !other_active_after_deactivation {
            self.activate_v2_membership_row(installation_id, user_id, now)
                .await?;
            return Ok(MembershipDeactivation::FinalMemberReserved);
        }
        let core = self
            .finish_v2_membership_removal(installation_id, user_id, now)
            .await?;
        let installation = self.reconstruct_v2_installation(&core).await?;
        self.put_installation(&installation, CasExpectation::Any)
            .await
            .map_err(SaveRowError::into_installation_error)?;
        Ok(MembershipDeactivation::MembershipRemoved(Box::new(
            installation,
        )))
    }

    async fn reconstruct_v2_installation(
        &self,
        core: &V2InstallationRecord,
    ) -> Result<ExtensionInstallation, ExtensionInstallationError> {
        if !core.is_visible() {
            return Err(invalid_installation_error(
                "a removed or leased v2 installation cannot be reconstructed as live",
            ));
        }
        let all_memberships = self.query_v2_memberships(&core.installation_id).await?;
        let all_bindings = self
            .query_v2_credential_bindings(&core.installation_id)
            .await?;
        Self::assemble_v2_installation(core, all_memberships, all_bindings)
    }

    /// Group every membership row under the shared prefix by its installation.
    ///
    /// One query for the whole collection instead of one per installation:
    /// listing is a hot read and the per-installation shape made it O(N)
    /// round trips.
    async fn query_v2_memberships_by_installation(
        &self,
    ) -> Result<HashMap<ExtensionInstallationId, Vec<V2MembershipRecord>>, ExtensionInstallationError>
    {
        let rows = query_all(&self.filesystem, &self.v2_memberships_root()?, &Filter::All).await?;
        let mut grouped: HashMap<ExtensionInstallationId, Vec<V2MembershipRecord>> = HashMap::new();
        for row in rows {
            let path = row.path.clone();
            let record = parse_v2_membership_entry(row.entry, &path)?;
            // Same integrity check the per-installation read makes: the body's
            // installation id must agree with the parent the row is filed under.
            // Match the full directory boundary, not a bare string prefix.
            // Today's row tokens are fixed-length hashes so a prefix collision
            // is unreachable, but that is a property of the path scheme rather
            // than of this check -- pin the boundary so the check stays exact
            // if the scheme ever changes.
            let expected_root = format!("{}/", self.v2_membership_root(&record.installation_id)?);
            if !path.as_str().starts_with(&expected_root) {
                return Err(invalid_installation_error(
                    "v2 membership row installation id did not match its parent",
                ));
            }
            grouped
                .entry(record.installation_id.clone())
                .or_default()
                .push(record);
        }
        Ok(grouped)
    }

    /// Credential-binding counterpart of
    /// [`Self::query_v2_memberships_by_installation`].
    async fn query_v2_credential_bindings_by_installation(
        &self,
    ) -> Result<
        HashMap<ExtensionInstallationId, Vec<V2CredentialBindingRecord>>,
        ExtensionInstallationError,
    > {
        let rows = query_all(
            &self.filesystem,
            &self.v2_credential_bindings_root()?,
            &Filter::All,
        )
        .await?;
        let mut grouped: HashMap<ExtensionInstallationId, Vec<V2CredentialBindingRecord>> =
            HashMap::new();
        for row in rows {
            let path = row.path.clone();
            let record = parse_v2_credential_binding_entry(row.entry, &path)?;
            let expected_root = format!(
                "{}/",
                self.v2_credential_binding_root(&record.installation_id)?
            );
            if !path.as_str().starts_with(&expected_root) {
                return Err(invalid_installation_error(
                    "v2 credential binding installation id did not match its parent",
                ));
            }
            grouped
                .entry(record.installation_id.clone())
                .or_default()
                .push(record);
        }
        Ok(grouped)
    }

    /// Rebuild one aggregate from a core row and its already-loaded children.
    ///
    /// Pure assembly: the caller decides whether the children came from a
    /// per-installation read or a batched one, so a list can load every child
    /// collection once.
    fn assemble_v2_installation(
        core: &V2InstallationRecord,
        all_memberships: Vec<V2MembershipRecord>,
        all_bindings: Vec<V2CredentialBindingRecord>,
    ) -> Result<ExtensionInstallation, ExtensionInstallationError> {
        let membership_updated_at = all_memberships.iter().map(|record| record.updated_at).max();
        let memberships = all_memberships
            .into_iter()
            .filter(V2MembershipRecord::is_active)
            .collect::<Vec<_>>();
        let owner = if core.legacy_tenant_owner {
            InstallationOwner::Tenant
        } else {
            InstallationOwner::users(
                memberships
                    .iter()
                    .map(|record| record.user_id.clone())
                    .collect(),
            )?
        };
        let binding_updated_at = all_bindings.iter().map(|record| record.updated_at).max();
        let mut bindings = all_bindings
            .into_iter()
            .filter(V2CredentialBindingRecord::is_active)
            .collect::<Vec<_>>();
        bindings.sort_by(|left, right| {
            left.position
                .cmp(&right.position)
                .then_with(|| left.credential_handle.cmp(&right.credential_handle))
        });
        let credential_bindings = bindings
            .iter()
            .map(|record| {
                ExtensionCredentialBinding::new(
                    record.credential_handle.clone(),
                    record.secret_handle.clone(),
                )
            })
            .collect::<Vec<_>>();
        let updated_at = membership_updated_at
            .into_iter()
            .chain(binding_updated_at)
            .fold(core.updated_at, std::cmp::max);
        ExtensionInstallation::from_persisted_parts(ExtensionInstallationPersistedParts {
            installation_id: core.installation_id.clone(),
            extension_id: core.extension_id.clone(),
            manifest_ref: core.manifest_ref(),
            credential_bindings,
            updated_at,
            owner,
        })
    }

    /// Load every visible installation with a constant number of queries.
    ///
    /// The normalized layout stores each aggregate's members and credential
    /// bindings as child rows, so reconstructing them one installation at a
    /// time costs `1 + 2N` round trips. That is invisible on a local disk and
    /// dominates on network-attached storage, so both child collections are
    /// read once and joined in memory instead.
    async fn query_v2_installations(
        &self,
        filter: &Filter,
    ) -> Result<Vec<ExtensionInstallation>, ExtensionInstallationError> {
        let rows = query_all(&self.filesystem, &self.v2_installations_root()?, filter).await?;
        let mut cores = Vec::new();
        for row in rows {
            let core = parse_v2_installation_entry(row.entry, &row.path)?;
            if core.is_visible() {
                cores.push(core);
            }
        }
        if cores.is_empty() {
            return Ok(Vec::new());
        }
        let mut memberships = self.query_v2_memberships_by_installation().await?;
        let mut bindings = self.query_v2_credential_bindings_by_installation().await?;
        let mut installations = Vec::with_capacity(cores.len());
        for core in &cores {
            installations.push(Self::assemble_v2_installation(
                core,
                memberships
                    .remove(&core.installation_id)
                    .unwrap_or_default(),
                bindings.remove(&core.installation_id).unwrap_or_default(),
            )?);
        }
        Ok(installations)
    }

    /// Import legacy two-row aggregates (manifest row + installation row)
    /// absent from v2 by joining each legacy installation with its legacy
    /// manifest. A legacy installation without its manifest could never have
    /// operated and fails the import loudly; an orphaned legacy manifest row
    /// with no installation is the legacy flow's durable removal-cleanup
    /// marker and imports as a cleanup-pending tombstone so the interrupted
    /// removal stays retryable after the migration.
    async fn bootstrap_v2_from_compatibility_rows(&self) -> Result<(), ExtensionInstallationError> {
        let mut installed_extensions = BTreeSet::new();
        let legacy_installations = self.query_installations(&Filter::All).await?;
        for installation in legacy_installations {
            installed_extensions.insert(installation.extension_id().clone());
            if self
                .load_v2_installation_record(installation.installation_id())
                .await?
                .is_some()
            {
                continue;
            }
            let Some((manifest, _)) = self
                .load_manifest_entry(installation.extension_id())
                .await?
            else {
                return Err(invalid_installation_error(format!(
                    "legacy installation {} has no legacy manifest row to import",
                    installation.installation_id()
                )));
            };
            self.put_v2_installation(&installation, Some(&manifest))
                .await?;
        }
        let legacy_manifest_rows =
            query_all(&self.filesystem, &self.manifests_root()?, &Filter::All).await?;
        for row in legacy_manifest_rows {
            let manifest = self.parse_manifest_entry(row.entry, &row.path)?;
            if installed_extensions.contains(manifest.extension_id()) {
                continue;
            }
            if self
                .load_v2_records_by_extension(manifest.extension_id())
                .await?
                .is_empty()
            {
                self.persist_removal_tombstone(manifest).await?;
            }
        }
        Ok(())
    }

    async fn repair_removed_v2_children(&self) -> Result<(), ExtensionInstallationError> {
        let installation_rows = query_all(
            &self.filesystem,
            &self.v2_installations_root()?,
            &Filter::All,
        )
        .await?;
        for row in installation_rows {
            let core = parse_v2_installation_entry(row.entry, &row.path)?;
            if !core.is_removed() {
                continue;
            }
            let removed_at = core.removed_at.unwrap_or(core.updated_at);
            for membership in self.query_v2_memberships(&core.installation_id).await? {
                if membership.is_active() {
                    self.deactivate_v2_membership_row(
                        &core.installation_id,
                        &membership.user_id,
                        removed_at,
                    )
                    .await?;
                }
            }
            for binding in self
                .query_v2_credential_bindings(&core.installation_id)
                .await?
            {
                if binding.is_active() {
                    self.deactivate_v2_credential_binding_row(
                        &core.installation_id,
                        &binding.credential_handle,
                        removed_at,
                    )
                    .await?;
                }
            }
        }
        Ok(())
    }

    async fn repair_interrupted_v2_leases(&self) -> Result<(), ExtensionInstallationError> {
        let rows = query_all(
            &self.filesystem,
            &self.v2_installations_root()?,
            &Filter::All,
        )
        .await?;
        for row in rows {
            let core = parse_v2_installation_entry(row.entry, &row.path)?;
            if core.lease.is_none() || core.is_removed() {
                continue;
            }
            if let Some((installation, _)) =
                self.load_installation_entry(&core.installation_id).await?
            {
                self.put_v2_installation(&installation, None).await?;
                continue;
            }
            // A missing compatibility snapshot is NOT proof of removal:
            // successful v2 writes deliberately tolerate failed projection
            // writes, so a live record can legitimately have no snapshot.
            // The child rows are the authority. Surviving active membership
            // (or a legacy tenant owner) means the record was live — clear
            // the lease over the children as they stand. Zero active members
            // can only mean a final removal already tombstoned every child,
            // so only then does the record roll forward to removed.
            let has_live_owner = core.legacy_tenant_owner
                || self
                    .query_v2_memberships(&core.installation_id)
                    .await?
                    .iter()
                    .any(V2MembershipRecord::is_active);
            if has_live_owner {
                self.clear_v2_lease(&core.installation_id).await?;
            } else {
                self.finish_v2_installation_removal(&core.installation_id, Utc::now())
                    .await?;
            }
        }
        Ok(())
    }

    /// Release a dangling lease without touching any other state, leaving the
    /// record live over its child rows as they stand. Startup lease recovery
    /// uses this when no compatibility snapshot exists to roll back to.
    async fn clear_v2_lease(
        &self,
        installation_id: &ExtensionInstallationId,
    ) -> Result<(), ExtensionInstallationError> {
        let path = self.v2_scoped_path(&self.v2_installation_path(installation_id)?)?;
        let installation_id = installation_id.clone();
        cas_update(
            self.scoped_filesystem.as_ref(),
            &ResourceScope::system(),
            &path,
            decode_v2_installation_body,
            entry_for_v2_installation,
            move |current: Option<V2InstallationRecord>| {
                let installation_id = installation_id.clone();
                async move {
                    let Some(mut record) = current else {
                        return Err(ExtensionInstallationError::InstallationNotFound {
                            installation_id,
                        });
                    };
                    if record.installation_id != installation_id {
                        return Err(invalid_installation_error(
                            "v2 installation body identity did not match its key",
                        ));
                    }
                    if record.lease.is_some() {
                        record.lease = None;
                        record.updated_at = Utc::now();
                    }
                    Ok(CasApply::new(record, ()))
                }
            },
        )
        .await
        .map_err(|error| map_extension_state_cas_error(error, "installation"))
    }

    async fn repair_compatibility_views(&self) -> Result<(), ExtensionInstallationError> {
        let installation_rows = query_all(
            &self.filesystem,
            &self.v2_installations_root()?,
            &Filter::All,
        )
        .await?;
        for row in installation_rows {
            let core = parse_v2_installation_entry(row.entry, &row.path)?;
            if core.lease.is_some() && !core.is_removed() {
                // A lease observed here was taken by a live peer between the
                // recovery pass and this one. Failing the whole store open
                // over one mid-mutation record would trade correctness for
                // availability; skip it — the holder releases it, or the
                // next startup recovers it.
                continue;
            }
            if core.is_removed() {
                if let Some((_, version)) =
                    self.load_installation_entry(&core.installation_id).await?
                {
                    self.delete_installation_row(&core.installation_id, version)
                        .await
                        .map_err(SaveRowError::into_installation_error)?;
                }
                if core.removal_cleanup_pending {
                    // The definition is still authoritative for cleanup
                    // retries, so its legacy projection stays in place.
                    let manifest = core.manifest.clone().into_manifest_record()?;
                    self.put_manifest(&manifest, CasExpectation::Any)
                        .await
                        .map_err(SaveRowError::into_installation_error)?;
                } else if let Some((_, version)) =
                    self.load_manifest_entry(&core.extension_id).await?
                {
                    self.delete_manifest_row(&core.extension_id, version)
                        .await
                        .map_err(SaveRowError::into_installation_error)?;
                }
            } else {
                let manifest = core.manifest.clone().into_manifest_record()?;
                self.put_manifest(&manifest, CasExpectation::Any)
                    .await
                    .map_err(SaveRowError::into_installation_error)?;
                let installation = self.reconstruct_v2_installation(&core).await?;
                self.put_installation(&installation, CasExpectation::Any)
                    .await
                    .map_err(SaveRowError::into_installation_error)?;
            }
        }
        Ok(())
    }
}

#[async_trait]
impl ExtensionInstallationStorePort for ExtensionInstallationStore {
    async fn list_manifests(
        &self,
    ) -> Result<Vec<ExtensionManifestRecord>, ExtensionInstallationError> {
        let rows = query_all(
            &self.filesystem,
            &self.v2_installations_root()?,
            &Filter::All,
        )
        .await?;
        let mut manifests = rows
            .into_iter()
            .map(|row| parse_v2_installation_entry(row.entry, &row.path))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            // The definition stays readable while a lease is held — an
            // in-flight membership mutation does not change what is pinned —
            // and while a removal-cleanup tombstone awaits convergence.
            .filter(V2InstallationRecord::manifest_is_authoritative)
            .map(|record| record.manifest.into_manifest_record())
            .collect::<Result<Vec<_>, _>>()?;
        manifests.sort_by(|a, b| a.extension_id().cmp(b.extension_id()));
        Ok(manifests)
    }

    async fn get_manifest(
        &self,
        extension_id: &ExtensionId,
    ) -> Result<Option<ExtensionManifestRecord>, ExtensionInstallationError> {
        let Some(record) = self
            .load_v2_records_by_extension(extension_id)
            .await?
            .into_iter()
            .find(V2InstallationRecord::manifest_is_authoritative)
        else {
            return Ok(None);
        };
        record.manifest.into_manifest_record().map(Some)
    }

    async fn persist_removal_tombstone(
        &self,
        manifest: ExtensionManifestRecord,
    ) -> Result<(), ExtensionInstallationError> {
        let installation_id =
            ExtensionInstallationId::new(manifest.extension_id().as_str().to_string())
                .map_err(invalid_installation_error)?;
        let path = self.v2_scoped_path(&self.v2_installation_path(&installation_id)?)?;
        let wire = WireManifestRecord::from(&manifest);
        let extension_id = manifest.extension_id().clone();
        cas_update(
            self.scoped_filesystem.as_ref(),
            &ResourceScope::system(),
            &path,
            decode_v2_installation_body,
            entry_for_v2_installation,
            move |current: Option<V2InstallationRecord>| {
                let installation_id = installation_id.clone();
                let extension_id = extension_id.clone();
                let wire = wire.clone();
                async move {
                    if current.as_ref().is_some_and(|record| {
                        record.installation_id != installation_id
                            || record.extension_id != extension_id
                    }) {
                        return Err(invalid_installation_error(
                            "v2 installation body identity did not match its key",
                        ));
                    }
                    if let Some(record) = current.as_ref()
                        && !record.is_removed()
                    {
                        // A leased record is a peer's in-flight mutation, not
                        // an orphan: surface the retryable class so the
                        // caller waits it out instead of treating a live
                        // extension as cleanup debris.
                        if record.lease.is_some() {
                            return Err(ExtensionInstallationError::MembershipMutationInProgress {
                                installation_id: record.installation_id.clone(),
                            });
                        }
                        return Err(ExtensionInstallationError::InvalidInstallation {
                            reason: format!("extension {extension_id} still has installations"),
                        });
                    }
                    let now = Utc::now();
                    let record = V2InstallationRecord {
                        schema_version: EXTENSION_STATE_V2_SCHEMA.to_string(),
                        installation_id,
                        extension_id,
                        manifest: wire,
                        legacy_tenant_owner: current
                            .as_ref()
                            .map(|record| record.legacy_tenant_owner)
                            .unwrap_or(false),
                        updated_at: now,
                        removed_at: Some(
                            current
                                .as_ref()
                                .and_then(|record| record.removed_at)
                                .unwrap_or(now),
                        ),
                        lease: None,
                        removal_cleanup_pending: true,
                    };
                    Ok(CasApply::new(record, ()))
                }
            },
        )
        .await
        .map_err(|error| map_extension_state_cas_error(error, "installation"))?;
        // silent-ok: the v2 tombstone (the authority) is committed; the legacy
        // manifest projection mirrors it best-effort and startup repair
        // converges it.
        let _ = self.put_manifest(&manifest, CasExpectation::Any).await;
        Ok(())
    }

    async fn upsert_manifest_and_installation(
        &self,
        manifest: ExtensionManifestRecord,
        installation: ExtensionInstallation,
    ) -> Result<(), ExtensionInstallationError> {
        validate_installation_against_one_manifest(&manifest, &installation)?;
        self.put_v2_installation(&installation, Some(&manifest))
            .await?;
        // silent-ok: the merged v2 record (the authority) is committed, so
        // the install has succeeded; failing on a compatibility-projection
        // write would make callers compensate against a live install and
        // leave a ghost. Startup repairs the projections from v2.
        let _ = self.put_manifest(&manifest, CasExpectation::Any).await;
        let _ = self
            .put_installation(&installation, CasExpectation::Any)
            .await;
        Ok(())
    }

    async fn list_installations(
        &self,
    ) -> Result<Vec<ExtensionInstallation>, ExtensionInstallationError> {
        let mut installations = self.query_v2_installations(&Filter::All).await?;
        installations.sort_by(|a, b| a.installation_id().cmp(b.installation_id()));
        Ok(installations)
    }

    async fn get_installation(
        &self,
        installation_id: &ExtensionInstallationId,
    ) -> Result<Option<ExtensionInstallation>, ExtensionInstallationError> {
        let Some((core, _)) = self.load_v2_installation_record(installation_id).await? else {
            return Ok(None);
        };
        if !core.is_visible() {
            return Ok(None);
        }
        self.reconstruct_v2_installation(&core).await.map(Some)
    }

    async fn upsert_installation(
        &self,
        installation: ExtensionInstallation,
    ) -> Result<(), ExtensionInstallationError> {
        // Validate against the record's own pinned definition — a removed
        // record retains it, which is what lets a reactivating upsert
        // revalidate without re-supplying the manifest. A row that does not
        // exist yet (a legacy-shaped additional installation of an already
        // pinned extension) falls back to the extension's authoritative
        // definition, which that new row then embeds.
        let (manifest, provide_manifest) = match self
            .load_v2_installation_record(installation.installation_id())
            .await?
        {
            Some((record, _)) => (record.manifest.into_manifest_record()?, false),
            None => (
                self.get_manifest(installation.extension_id())
                    .await?
                    .ok_or_else(|| ExtensionInstallationError::UnknownManifest {
                        extension_id: installation.extension_id().clone(),
                    })?,
                true,
            ),
        };
        validate_installation_against_one_manifest(&manifest, &installation)?;
        self.put_v2_installation(&installation, provide_manifest.then_some(&manifest))
            .await?;
        // silent-ok: v2 is authoritative and committed; see
        // upsert_manifest_and_installation for why the projection write must
        // not fail the operation. Startup repairs the projection from v2.
        let _ = self
            .put_installation(&installation, CasExpectation::Any)
            .await;
        Ok(())
    }

    async fn activate_membership(
        &self,
        installation_id: &ExtensionInstallationId,
        user_id: &UserId,
    ) -> Result<ExtensionInstallation, ExtensionInstallationError> {
        let Some((core, _)) = self.load_v2_installation_record(installation_id).await? else {
            return Err(ExtensionInstallationError::InstallationNotFound {
                installation_id: installation_id.clone(),
            });
        };
        if core.is_removed() {
            return Err(ExtensionInstallationError::InstallationNotFound {
                installation_id: installation_id.clone(),
            });
        }
        if core.lease.is_some() {
            return Err(ExtensionInstallationError::MembershipMutationInProgress {
                installation_id: installation_id.clone(),
            });
        }
        let now = Utc::now();
        let changed = self
            .activate_v2_membership_row(installation_id, user_id, now)
            .await?;
        let Some((observed_core, _)) = self.load_v2_installation_record(installation_id).await?
        else {
            if changed {
                self.deactivate_v2_membership_row(installation_id, user_id, now)
                    .await?;
            }
            return Err(ExtensionInstallationError::InstallationNotFound {
                installation_id: installation_id.clone(),
            });
        };
        if !observed_core.is_visible() {
            // Deliberately roll the new row back only for a tombstoned
            // record. While a removal lease is held, unconditionally
            // deactivating our row can interleave with the lease holder's
            // member-count recheck and strand a live record with zero
            // members. Leaving the row active is the safe at-least-once
            // outcome: a final removal tombstones every active row, and the
            // caller's transient error invites the retry that converges.
            if changed && observed_core.is_removed() {
                self.deactivate_v2_membership_row(installation_id, user_id, now)
                    .await?;
            }
            return if observed_core.is_removed() {
                Err(ExtensionInstallationError::InstallationNotFound {
                    installation_id: installation_id.clone(),
                })
            } else {
                Err(ExtensionInstallationError::MembershipMutationInProgress {
                    installation_id: installation_id.clone(),
                })
            };
        }
        let core = self
            .canonicalize_v2_installation_owner(installation_id, now)
            .await?;
        let installation = self.reconstruct_v2_installation(&core).await?;
        // silent-ok: the membership row (the authority) is committed; a
        // projection-write failure must not report the join as failed.
        // Startup repairs the projection from v2.
        let _ = self
            .put_installation(&installation, CasExpectation::Any)
            .await;
        Ok(installation)
    }

    async fn deactivate_membership(
        &self,
        installation_id: &ExtensionInstallationId,
        user_id: &UserId,
    ) -> Result<MembershipDeactivation, ExtensionInstallationError> {
        let Some((current_core, _)) = self.load_v2_installation_record(installation_id).await?
        else {
            return Err(ExtensionInstallationError::InstallationNotFound {
                installation_id: installation_id.clone(),
            });
        };
        if current_core.legacy_tenant_owner {
            return Err(ExtensionInstallationError::LegacyTenantOwnerNotCanonicalized);
        }
        if !current_core.is_visible() {
            return Err(ExtensionInstallationError::MembershipMutationInProgress {
                installation_id: installation_id.clone(),
            });
        }
        let prior = self.reconstruct_v2_installation(&current_core).await?;
        let now = Utc::now();
        if let Err(error) = self
            .reserve_v2_membership_removal(installation_id, user_id, now)
            .await
        {
            if !matches!(error, ExtensionInstallationError::StoreUnavailable { .. }) {
                return Err(error);
            }
            let v2_restored = self.write_v2_installation_components(&prior, None).await;
            let compatibility_restored = self.put_installation(&prior, CasExpectation::Any).await;
            if v2_restored.is_err() || compatibility_restored.is_err() {
                return Err(store_unavailable_error(
                    "membership reservation failed and its prior installation could not be restored",
                ));
            }
            return Err(error);
        }
        match self
            .deactivate_reserved_v2_membership(installation_id, user_id, now)
            .await
        {
            Ok(result) => Ok(result),
            Err(error) => {
                let v2_restored = self.write_v2_installation_components(&prior, None).await;
                let compatibility_restored =
                    self.put_installation(&prior, CasExpectation::Any).await;
                if v2_restored.is_err() || compatibility_restored.is_err() {
                    return Err(store_unavailable_error(
                        "membership mutation failed and its prior installation could not be restored",
                    ));
                }
                Err(error)
            }
        }
    }

    async fn delete_installation(
        &self,
        installation_id: &ExtensionInstallationId,
    ) -> Result<(), ExtensionInstallationError> {
        let now = Utc::now();
        self.begin_v2_installation_removal(installation_id, now)
            .await?;
        for membership in self.query_v2_memberships(installation_id).await? {
            if membership.is_active() {
                self.deactivate_v2_membership_row(installation_id, &membership.user_id, now)
                    .await?;
            }
        }
        for binding in self.query_v2_credential_bindings(installation_id).await? {
            if binding.is_active() {
                self.deactivate_v2_credential_binding_row(
                    installation_id,
                    &binding.credential_handle,
                    now,
                )
                .await?;
            }
        }
        self.finish_v2_installation_removal(installation_id, now)
            .await?;
        // The legacy manifest projection deliberately stays: the tombstone is
        // cleanup-pending and its definition remains authoritative until
        // `delete_manifest` marks convergence and retires both.
        if let Some((_, version)) = self.load_installation_entry(installation_id).await? {
            self.delete_installation_row(installation_id, version)
                .await
                .map_err(SaveRowError::into_installation_error)?;
        }
        Ok(())
    }

    /// The definition is tombstoned by `delete_installation`; this method is
    /// the convergence marker — it errors while the definition is still live,
    /// clears the cleanup-pending state, and retires the legacy manifest
    /// projection.
    async fn delete_manifest(
        &self,
        extension_id: &ExtensionId,
    ) -> Result<(), ExtensionInstallationError> {
        let records = self.load_v2_records_by_extension(extension_id).await?;
        if records.is_empty() {
            return Err(ExtensionInstallationError::ManifestNotFound {
                extension_id: extension_id.clone(),
            });
        }
        if let Some(leased) = records
            .iter()
            .find(|record| record.lease.is_some() && !record.is_removed())
        {
            return Err(ExtensionInstallationError::MembershipMutationInProgress {
                installation_id: leased.installation_id.clone(),
            });
        }
        if records.iter().any(|record| !record.is_removed()) {
            return Err(ExtensionInstallationError::InvalidInstallation {
                reason: format!("extension {extension_id} still has installations"),
            });
        }
        for record in records
            .iter()
            .filter(|record| record.removal_cleanup_pending)
        {
            self.mark_v2_removal_converged(&record.installation_id)
                .await?;
        }
        if let Some((_, version)) = self.load_manifest_entry(extension_id).await? {
            self.delete_manifest_row(extension_id, version)
                .await
                .map_err(SaveRowError::into_installation_error)?;
        }
        Ok(())
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct WireManifestRecord {
    raw_toml: String,
    source: WireManifestSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    resolved: Option<ResolvedExtensionManifest>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    manifest_hash: Option<ManifestHash>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    removal_cleanup_requirements: Vec<ExtensionRemovalCleanupRequirement>,
}

impl WireManifestRecord {
    fn into_manifest_record(self) -> Result<ExtensionManifestRecord, ExtensionInstallationError> {
        let resolved = self.resolved.ok_or_else(|| {
            invalid_installation_error("extension manifest row was not resolved during store load")
        })?;
        ExtensionManifestRecord::from_resolved(
            self.raw_toml,
            self.source.into_manifest_source(),
            resolved,
            self.manifest_hash,
        )
        .map(|record| record.with_removal_cleanup_requirements(self.removal_cleanup_requirements))
    }
}

impl From<&ExtensionManifestRecord> for WireManifestRecord {
    fn from(record: &ExtensionManifestRecord) -> Self {
        Self {
            raw_toml: record.raw_toml().to_string(),
            source: WireManifestSource::from_manifest_source(record.manifest().source),
            resolved: Some(record.resolved().clone()),
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

fn entry_for_v2_installation(
    record: &V2InstallationRecord,
) -> Result<Entry, ExtensionInstallationError> {
    entry_for_v2_record(
        INSTALLATION_RECORD_KIND_V2,
        record,
        [
            (
                "installation_id",
                IndexValue::Text(record.installation_id.as_str().to_string()),
            ),
            (
                "extension_id",
                IndexValue::Text(record.extension_id.as_str().to_string()),
            ),
        ],
    )
}

fn entry_for_v2_membership(
    record: &V2MembershipRecord,
) -> Result<Entry, ExtensionInstallationError> {
    entry_for_v2_record(
        MEMBERSHIP_RECORD_KIND_V2,
        record,
        [
            (
                "installation_id",
                IndexValue::Text(record.installation_id.as_str().to_string()),
            ),
            (
                "user_id",
                IndexValue::Text(record.user_id.as_str().to_string()),
            ),
        ],
    )
}

fn entry_for_v2_credential_binding(
    record: &V2CredentialBindingRecord,
) -> Result<Entry, ExtensionInstallationError> {
    entry_for_v2_record(
        CREDENTIAL_BINDING_RECORD_KIND_V2,
        record,
        [
            (
                "installation_id",
                IndexValue::Text(record.installation_id.as_str().to_string()),
            ),
            (
                "credential_handle",
                IndexValue::Text(record.credential_handle.as_str().to_string()),
            ),
        ],
    )
}

fn entry_for_v2_record<T, const N: usize>(
    kind: &'static str,
    record: &T,
    indexed: [(&'static str, IndexValue); N],
) -> Result<Entry, ExtensionInstallationError>
where
    T: Serialize,
{
    let payload = serde_json::to_value(record).map_err(invalid_installation_error)?;
    let mut entry =
        Entry::record(record_kind(kind)?, &payload).map_err(invalid_installation_error)?;
    for (key, value) in indexed {
        entry = entry.with_indexed(index_key(key)?, value);
    }
    Ok(entry)
}

fn parse_v2_installation_entry(
    entry: Entry,
    path: &VirtualPath,
) -> Result<V2InstallationRecord, ExtensionInstallationError> {
    parse_v2_entry(entry, path, INSTALLATION_RECORD_KIND_V2, "installation")
}

fn parse_v2_membership_entry(
    entry: Entry,
    path: &VirtualPath,
) -> Result<V2MembershipRecord, ExtensionInstallationError> {
    parse_v2_entry(entry, path, MEMBERSHIP_RECORD_KIND_V2, "membership")
}

fn decode_v2_installation_body(
    body: &[u8],
) -> Result<V2InstallationRecord, ExtensionInstallationError> {
    decode_v2_record_body(body, "installation")
}

fn decode_v2_membership_body(
    body: &[u8],
) -> Result<V2MembershipRecord, ExtensionInstallationError> {
    decode_v2_record_body(body, "membership")
}

fn parse_v2_credential_binding_entry(
    entry: Entry,
    path: &VirtualPath,
) -> Result<V2CredentialBindingRecord, ExtensionInstallationError> {
    parse_v2_entry(
        entry,
        path,
        CREDENTIAL_BINDING_RECORD_KIND_V2,
        "credential binding",
    )
}

fn decode_v2_credential_binding_body(
    body: &[u8],
) -> Result<V2CredentialBindingRecord, ExtensionInstallationError> {
    decode_v2_record_body(body, "credential binding")
}

fn decode_v2_record_body<T>(
    body: &[u8],
    label: &'static str,
) -> Result<T, ExtensionInstallationError>
where
    T: serde::de::DeserializeOwned,
{
    let value: serde_json::Value =
        serde_json::from_slice(body).map_err(invalid_installation_error)?;
    if value
        .get("schema_version")
        .and_then(serde_json::Value::as_str)
        != Some(EXTENSION_STATE_V2_SCHEMA)
    {
        return Err(invalid_installation_error(format!(
            "v2 extension {label} row had unsupported schema version"
        )));
    }
    serde_json::from_value(value).map_err(invalid_installation_error)
}

fn parse_v2_entry<T>(
    entry: Entry,
    path: &VirtualPath,
    expected_kind: &'static str,
    label: &'static str,
) -> Result<T, ExtensionInstallationError>
where
    T: serde::de::DeserializeOwned,
{
    ensure_entry_kind(&entry, expected_kind, path)?;
    let record: T = entry
        .parse_json()
        .map_err(|error| corrupt_row("deserialize v2 extension state row", path, error))?;
    let value = serde_json::from_slice::<serde_json::Value>(&entry.body)
        .map_err(|error| corrupt_row("inspect v2 extension state schema", path, error))?;
    if value
        .get("schema_version")
        .and_then(serde_json::Value::as_str)
        != Some(EXTENSION_STATE_V2_SCHEMA)
    {
        return Err(invalid_installation_error(format!(
            "v2 extension {label} row had unsupported schema version"
        )));
    }
    Ok(record)
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

fn map_extension_state_cas_error(
    error: CasUpdateError<ExtensionInstallationError>,
    record: &'static str,
) -> ExtensionInstallationError {
    match error {
        CasUpdateError::Apply(error) => error,
        CasUpdateError::Timeout => {
            store_unavailable_error(format!("v2 extension {record} CAS update timed out"))
        }
        CasUpdateError::RetriesExhausted => {
            store_unavailable_error(format!("v2 extension {record} changed repeatedly"))
        }
        CasUpdateError::CasUnsupported => {
            store_unavailable_error(format!("v2 extension {record} store does not support CAS"))
        }
        CasUpdateError::Backend(_) => {
            store_unavailable_error(format!("v2 extension {record} backend operation failed"))
        }
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
    async fn load_migrates_pre_resolved_manifest_rows_before_projection() {
        let backend = Arc::new(InMemoryBackend::new());
        let root = VirtualPath::new("/system/extensions/.installations/migration-test")
            .expect("valid root");
        let store = ExtensionInstallationStore::load_at(
            backend.clone(),
            root.clone(),
            HostPortCatalog::empty(),
            capability_provider_contracts(),
        )
        .await
        .expect("initial store");
        let record = manifest_record("legacy-fixture", Some("hash-legacy"));
        let path = store
            .manifest_path(record.extension_id())
            .expect("manifest path");
        let wire = WireManifestRecord {
            raw_toml: record.raw_toml().to_string(),
            source: WireManifestSource::from_manifest_source(record.manifest().source),
            resolved: None,
            manifest_hash: record.manifest_hash().cloned(),
            removal_cleanup_requirements: Vec::new(),
        };
        let payload = serde_json::to_value(wire).expect("legacy wire payload");
        let entry = Entry::record(
            record_kind(MANIFEST_RECORD_KIND).expect("record kind"),
            &payload,
        )
        .expect("legacy manifest entry")
        .with_indexed(
            index_key("extension_id").expect("index key"),
            IndexValue::Text(record.extension_id().as_str().to_string()),
        )
        .with_indexed(
            index_key("manifest_source").expect("index key"),
            IndexValue::Text("host_bundled".to_string()),
        );
        backend
            .put(&path, entry, CasExpectation::Any)
            .await
            .expect("seed legacy row");
        // The merged v2 record imports legacy state as a manifest+installation
        // pair; an orphan legacy manifest is a v1 crash artifact and stays
        // uninstalled, so pair the seeded wire with its installation row.
        store
            .put_installation(
                &installation("legacy-fixture", Some("hash-legacy")),
                CasExpectation::Any,
            )
            .await
            .expect("seed legacy installation row");
        drop(store);

        let reopened = ExtensionInstallationStore::load_at(
            backend,
            root,
            HostPortCatalog::empty(),
            capability_provider_contracts(),
        )
        .await
        .expect("migration succeeds");
        let loaded = reopened
            .get_manifest(record.extension_id())
            .await
            .expect("load migrated row")
            .expect("migrated row exists");
        assert_eq!(loaded.resolved(), record.resolved());
    }

    #[tokio::test]
    async fn delete_manifest_rejects_active_installations() {
        let store = installation_store().await;
        let manifest = manifest_record("fixture", Some("hash-1"));
        let extension_id = manifest.extension_id().clone();
        store
            .upsert_manifest_and_installation(manifest, installation("fixture", Some("hash-1")))
            .await
            .expect("install fixture");

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

    async fn installation_store() -> ExtensionInstallationStore {
        ExtensionInstallationStore::load_at(
            Arc::new(InMemoryBackend::new()),
            VirtualPath::new("/system/extensions/.installations/test").expect("valid root"),
            HostPortCatalog::empty(),
            capability_provider_contracts(),
        )
        .await
        .expect("filesystem store")
    }

    fn capability_provider_contracts() -> crate::HostApiContractRegistry {
        let mut contracts = crate::HostApiContractRegistry::new();
        contracts
            .register(std::sync::Arc::new(
                crate::CapabilityProviderHostApiContract::new().expect("contract"),
            ))
            .expect("register capability provider contract");
        contracts
    }

    fn manifest_record(extension_id: &str, hash: Option<&str>) -> ExtensionManifestRecord {
        ExtensionManifestRecord::from_toml(
            manifest_toml(extension_id),
            ManifestSource::HostBundled,
            &HostPortCatalog::empty(),
            hash.map(|value| ManifestHash::new(value).expect("hash")),
            &capability_provider_contracts(),
        )
        .expect("manifest record")
    }

    fn installation(extension_id: &str, hash: Option<&str>) -> ExtensionInstallation {
        let extension_id = ExtensionId::new(extension_id.to_string()).expect("extension id");
        ExtensionInstallation::new(
            ExtensionInstallationId::new(extension_id.as_str().to_string())
                .expect("installation id"),
            extension_id.clone(),
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

    #[test]
    fn caller_membership_join_and_leave_are_idempotent_domain_transitions() {
        let alice = ironclaw_host_api::UserId::new("alice").expect("user id");
        let bob = ironclaw_host_api::UserId::new("bob").expect("user id");

        assert_eq!(
            InstallationOwner::Tenant
                .without_member(&alice)
                .expect_err("legacy tenant rows must be narrowed before removal"),
            ExtensionInstallationError::LegacyTenantOwnerNotCanonicalized,
            "a caller must never tear down a legacy shared row directly"
        );

        let alice_only = InstallationOwner::Tenant
            .joined_by(&alice)
            .expect("legacy owner narrows")
            .expect("owner changes");
        assert_eq!(alice_only, InstallationOwner::user(alice.clone()));
        assert_eq!(
            alice_only.joined_by(&alice).expect("same-member retry"),
            None,
            "joining an existing member must not rewrite the row"
        );

        let alice_and_bob = alice_only
            .joined_by(&bob)
            .expect("Bob joins")
            .expect("owner changes");
        assert!(alice_and_bob.visible_to(&alice));
        assert!(alice_and_bob.visible_to(&bob));

        let bob_only = alice_and_bob
            .without_member(&alice)
            .expect("Alice leaves")
            .expect("Bob remains");
        assert!(!bob_only.visible_to(&alice));
        assert!(bob_only.visible_to(&bob));
        assert_eq!(
            bob_only.without_member(&bob).expect("Bob leaves"),
            None,
            "the last member tears down the aggregate"
        );
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

[[host_api]]
id = "ironclaw.capability_provider/v1"
section = "capability_provider.tools"

[capability_provider.tools]

[[capability_provider.tools.capabilities]]
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
    #[error("invalid extension manifest: {reason}")]
    InvalidManifest { reason: String },
    #[error("invalid {field}: {reason}")]
    InvalidValue { field: &'static str, reason: String },
    #[error("installation owner member set must not be empty")]
    EmptyOwnerMembers,
    #[error("legacy tenant installation owner must be canonicalized before member removal")]
    LegacyTenantOwnerNotCanonicalized,
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
    #[error("installation {installation_id} has another membership mutation in progress")]
    MembershipMutationInProgress {
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
