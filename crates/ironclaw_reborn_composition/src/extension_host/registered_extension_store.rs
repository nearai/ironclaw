//! Owner-scoped storage and catalog composition for user-registered MCP
//! extensions. Descriptors live at
//! `/system/extensions/registered/<owner>/<id>/manifest.toml`.
//!
//! **Boot-leak invariant (do not weaken):** the shared, process-wide
//! `AvailableExtensionCatalog` must never contain a `UserRegistered` package.
//! Its `search`/`resolve` do no owner filtering, so a registered package
//! reachable through it is installable by any owner. Every helper here reads
//! the owner overlay separately and merges at the call site.

use std::collections::BTreeSet;

use ironclaw_extensions::{
    ExtensionActivationState, ExtensionInstallationError, ExtensionInstallationStore,
    ExtensionManifestRecord, ExtensionRuntime, ManifestSource,
};
use ironclaw_filesystem::{CasExpectation, Entry, FileType, FilesystemError, RootFilesystem};
use ironclaw_host_api::{CapabilityId, ExtensionId, UserId, VirtualPath, sha256_digest_token};
use ironclaw_product_workflow::{LifecyclePackageRef, ProductWorkflowError};
use serde::{Deserialize, Serialize};

use crate::extension_host::available_extensions::{
    AvailableExtensionCatalog, AvailableExtensionPackage, is_internal_extension_package_ref,
    load_filesystem_packages, package_matches_search,
};
use crate::extension_host::mcp::HostedMcpEndpoint;

const REGISTERED_ROOT: &str = "/system/extensions/registered";
const REGISTERED_CLEANUP_ROOT: &str = "/system/extensions/registered-cleanup";
const DISCOVERED_CAPABILITY_IDS_SCHEMA_VERSION: &str =
    "registered_mcp.discovered_capability_ids.v1";
const DISCOVERED_CAPABILITY_IDS_LIMIT: usize = 128;
const DISCOVERED_CAPABILITY_IDS_MAX_BYTES: usize = 128 * 1024;
const REGISTERED_OWNER_DESCRIPTOR_LIMIT: usize = 32;
const REGISTERED_MANIFEST_VERSION: &str = "0.1.0";
const REGISTERED_MANIFEST_DESCRIPTION: &str = "User-registered hosted MCP server";

/// True for a package owned by a user's registered store. Such a package must
/// never be materialized under the shared `/system/extensions/<id>/` directory:
/// the next boot's catalog scan would re-adopt it as a first-party entry,
/// reopening the boot leak. Both writers in `extension_lifecycle.rs` (install
/// and restore) gate on this.
pub(crate) fn is_owner_registered(source: &ManifestSource) -> bool {
    matches!(source, ManifestSource::UserRegistered { .. })
}

/// T3-iso owner filter: a `UserRegistered` manifest is visible only to its
/// own owner; every other source is visible to any caller (including a
/// caller with no resolved identity).
pub(crate) fn manifest_visible_to_caller(source: &ManifestSource, caller: Option<&UserId>) -> bool {
    source.visible_to_caller(caller)
}

/// AC2: the set of extension ids that are BOTH enabled AND visible to
/// `owner` — the model-visible-capability filter
/// (`active_model_visible_capabilities`) intersects the shared registry
/// against this set, so a disabled or cross-owner `UserRegistered` extension's
/// capabilities never reach the model's toolbox. Deliberately NOT used by the
/// operator-tool-config surface below: `builtin.*` capabilities are never
/// installation-tracked, so an intersection against `list_enabled_installations()`
/// would silently drop them from Settings > Tools (see
/// `operator_config_excluded_extension_ids`'s doc for that surface's
/// default-allow design instead).
pub(crate) async fn owner_visible_enabled_extension_ids(
    installation_store: &dyn ExtensionInstallationStore,
    owner: &UserId,
) -> Result<BTreeSet<ExtensionId>, ExtensionInstallationError> {
    let enabled_ids: BTreeSet<ExtensionId> = installation_store
        .list_enabled_installations()
        .await?
        .into_iter()
        .map(|installation| installation.extension_id().clone())
        .collect();
    let owner_visible_ids: BTreeSet<ExtensionId> = installation_store
        .list_manifests()
        .await?
        .into_iter()
        .filter(|record| manifest_visible_to_caller(&record.manifest().source, Some(owner)))
        .map(|record| record.extension_id().clone())
        .collect();
    Ok(enabled_ids
        .intersection(&owner_visible_ids)
        .cloned()
        .collect())
}

/// Correction 10: the set of extension ids the operator-tool-config surface
/// (`ActiveRegistryOperatorToolCatalog`) must HIDE from `caller` — a
/// default-ALLOW exclusion list, unlike AC2's default-deny intersection.
/// `builtin.*` and other never-installation-tracked capabilities carry no
/// manifest or installation record at all, so they are never added here and
/// stay visible to every caller, matching Settings > Tools' pre-T3-iso
/// behavior for them. Only two things get excluded: a `UserRegistered`
/// manifest owned by someone else, and any tracked installation that is not
/// `Enabled` (disabled/quarantined).
pub(crate) async fn operator_config_excluded_extension_ids(
    installation_store: &dyn ExtensionInstallationStore,
    caller: &UserId,
) -> Result<BTreeSet<ExtensionId>, ExtensionInstallationError> {
    let mut excluded = BTreeSet::new();
    for record in installation_store.list_manifests().await? {
        if !manifest_visible_to_caller(&record.manifest().source, Some(caller)) {
            excluded.insert(record.extension_id().clone());
        }
    }
    for installation in installation_store.list_installations().await? {
        if installation.activation_state() != ExtensionActivationState::Enabled {
            excluded.insert(installation.extension_id().clone());
        }
    }
    Ok(excluded)
}

/// Owner-scoped read access to user-registered extension manifests.
pub(crate) struct RegisteredExtensionStore;

impl RegisteredExtensionStore {
    fn registered_root() -> Result<VirtualPath, ProductWorkflowError> {
        VirtualPath::new(REGISTERED_ROOT).map_err(map_binding_error)
    }

    /// `/system/extensions/registered/<owner>` — the directory
    /// [`load_filesystem_packages`] lists for one owner's registered set.
    fn owner_root(owner: &UserId) -> Result<VirtualPath, ProductWorkflowError> {
        VirtualPath::new(format!("{REGISTERED_ROOT}/{}", owner.as_str())).map_err(map_binding_error)
    }

    fn descriptor_root(
        owner: &UserId,
        extension_id: &ExtensionId,
    ) -> Result<VirtualPath, ProductWorkflowError> {
        VirtualPath::new(format!(
            "{REGISTERED_ROOT}/{}/{}",
            owner.as_str(),
            extension_id.as_str()
        ))
        .map_err(map_binding_error)
    }

    fn manifest_path(
        owner: &UserId,
        extension_id: &ExtensionId,
    ) -> Result<VirtualPath, ProductWorkflowError> {
        VirtualPath::new(format!(
            "{REGISTERED_ROOT}/{}/{}/manifest.toml",
            owner.as_str(),
            extension_id.as_str()
        ))
        .map_err(map_binding_error)
    }

    /// Durable unregister cleanup intent. This path must remain outside the
    /// descriptor subtree so deleting
    /// `/system/extensions/registered/<owner>/<extension_id>` cannot erase the
    /// capability ids needed to revoke approval authority on a retry.
    fn discovered_capability_ids_path(
        owner: &UserId,
        extension_id: &ExtensionId,
    ) -> Result<VirtualPath, ProductWorkflowError> {
        VirtualPath::new(format!(
            "{REGISTERED_CLEANUP_ROOT}/{}/{}/discovered-capability-ids.json",
            owner.as_str(),
            extension_id.as_str()
        ))
        .map_err(map_binding_error)
    }

    /// Load the current discovered capability-id set. Absence means no cleanup
    /// intent. Stored bytes are schema-, owner-, extension-, size-, and
    /// canonical-order validated before any id can be used as an approval key.
    pub(crate) async fn load_discovered_capability_ids<F>(
        fs: &F,
        owner: &UserId,
        extension_id: &ExtensionId,
    ) -> Result<Vec<CapabilityId>, ProductWorkflowError>
    where
        F: RootFilesystem + ?Sized,
    {
        let path = Self::discovered_capability_ids_path(owner, extension_id)?;
        let versioned = match fs.get(&path).await {
            Ok(Some(versioned)) => versioned,
            Ok(None)
            | Err(FilesystemError::NotFound { .. })
            | Err(FilesystemError::MountNotFound { .. }) => return Ok(Vec::new()),
            Err(error) => {
                return Err(ProductWorkflowError::Transient {
                    reason: format!("failed to load registered MCP capability inventory: {error}"),
                });
            }
        };
        if versioned.entry.body.len() > DISCOVERED_CAPABILITY_IDS_MAX_BYTES {
            return Err(invalid_stored_inventory(format!(
                "inventory exceeds {DISCOVERED_CAPABILITY_IDS_MAX_BYTES} bytes"
            )));
        }
        let stored: StoredDiscoveredCapabilityIds = serde_json::from_slice(&versioned.entry.body)
            .map_err(|_error| {
            invalid_stored_inventory("inventory JSON is invalid".to_string())
        })?;
        if stored.schema_version != DISCOVERED_CAPABILITY_IDS_SCHEMA_VERSION {
            return Err(invalid_stored_inventory(
                "unsupported schema version".to_string(),
            ));
        }
        if &stored.owner != owner || &stored.extension_id != extension_id {
            return Err(invalid_stored_inventory(
                "stored owner or extension does not match the inventory path".to_string(),
            ));
        }
        let canonical = canonical_discovered_capability_ids(extension_id, &stored.capability_ids)
            .map_err(invalid_stored_inventory)?;
        if canonical != stored.capability_ids {
            return Err(invalid_stored_inventory(
                "capability ids are not sorted and duplicate-free".to_string(),
            ));
        }
        Ok(canonical)
    }

    /// Atomically replace one owner's complete current discovered-id set.
    ///
    /// The lifecycle aggregate serializes this call with its existing
    /// operation lock. A single filesystem `put` is the atomic replacement
    /// boundary; no read-modify-write or per-capability files are involved.
    pub(crate) async fn replace_discovered_capability_ids<F>(
        fs: &F,
        owner: &UserId,
        extension_id: &ExtensionId,
        capability_ids: &[CapabilityId],
    ) -> Result<(), ProductWorkflowError>
    where
        F: RootFilesystem + ?Sized,
    {
        let capability_ids = canonical_discovered_capability_ids(extension_id, capability_ids)
            .map_err(|reason| ProductWorkflowError::InvalidBindingRequest { reason })?;
        let stored = StoredDiscoveredCapabilityIds {
            schema_version: DISCOVERED_CAPABILITY_IDS_SCHEMA_VERSION.to_string(),
            owner: owner.clone(),
            extension_id: extension_id.clone(),
            capability_ids,
        };
        let bytes = serde_json::to_vec_pretty(&stored).map_err(|error| {
            ProductWorkflowError::Transient {
                reason: format!("failed to serialize registered MCP capability inventory: {error}"),
            }
        })?;
        if bytes.len() > DISCOVERED_CAPABILITY_IDS_MAX_BYTES {
            return Err(ProductWorkflowError::InvalidBindingRequest {
                reason: format!(
                    "registered MCP capability inventory exceeds {DISCOVERED_CAPABILITY_IDS_MAX_BYTES} bytes"
                ),
            });
        }
        let path = Self::discovered_capability_ids_path(owner, extension_id)?;
        fs.put(&path, Entry::bytes(bytes), CasExpectation::Any)
            .await
            .map(|_| ())
            .map_err(|error| ProductWorkflowError::Transient {
                reason: format!("failed to replace registered MCP capability inventory: {error}"),
            })
    }

    /// Delete cleanup intent after every other unregister step succeeds.
    /// Missing intent is success so a completed unregister remains retry-safe.
    pub(crate) async fn delete_discovered_capability_ids<F>(
        fs: &F,
        owner: &UserId,
        extension_id: &ExtensionId,
    ) -> Result<(), ProductWorkflowError>
    where
        F: RootFilesystem + ?Sized,
    {
        let path = Self::discovered_capability_ids_path(owner, extension_id)?;
        match fs.delete(&path).await {
            Ok(())
            | Err(FilesystemError::NotFound { .. })
            | Err(FilesystemError::MountNotFound { .. }) => Ok(()),
            Err(error) => Err(ProductWorkflowError::Transient {
                reason: format!("failed to delete registered MCP capability inventory: {error}"),
            }),
        }
    }

    pub(crate) fn mint_hosted_mcp_extension_id(
        owner: &UserId,
        url: &str,
    ) -> Result<ExtensionId, ProductWorkflowError> {
        let normalized_url = normalized_hosted_mcp_url(url)?;
        let mut input = Vec::with_capacity(owner.as_str().len() + 1 + normalized_url.len());
        input.extend_from_slice(owner.as_str().as_bytes());
        input.push(0x1f);
        input.extend_from_slice(normalized_url.as_bytes());
        let digest = sha256_digest_token(&input);
        let suffix = digest
            .strip_prefix("sha256:")
            .unwrap_or(digest.as_str())
            .chars()
            .take(16)
            .collect::<String>();
        ExtensionId::new(format!("mcp-{suffix}")).map_err(map_binding_error)
    }

    pub(crate) fn synthesize_hosted_mcp_manifest_toml(
        owner: &UserId,
        input: &RegisterHostedMcpDescriptorInput,
    ) -> Result<RegisteredHostedMcpDescriptorDraft, ProductWorkflowError> {
        validate_registered_name(&input.name)?;
        let normalized_url = normalized_hosted_mcp_url(&input.url)?;
        let extension_id = Self::mint_hosted_mcp_extension_id(owner, &input.url)?;
        let dto = RegisteredHostedMcpManifestDto {
            schema_version: ironclaw_extensions::MANIFEST_SCHEMA_VERSION.to_string(),
            id: extension_id.as_str().to_string(),
            name: input.name.trim().to_string(),
            version: REGISTERED_MANIFEST_VERSION.to_string(),
            description: REGISTERED_MANIFEST_DESCRIPTION.to_string(),
            trust: "third_party".to_string(),
            runtime: RegisteredHostedMcpRuntimeDto {
                kind: "mcp".to_string(),
                transport: "http".to_string(),
                url: normalized_url.clone(),
            },
        };
        let manifest_toml =
            toml::to_string(&dto).map_err(|error| ProductWorkflowError::InvalidBindingRequest {
                reason: format!("failed to serialize registered extension manifest: {error}"),
            })?;
        let manifest_record = parse_registered_manifest(owner, manifest_toml.clone(), None)?;
        #[cfg(not(test))]
        let _ = manifest_record;
        Ok(RegisteredHostedMcpDescriptorDraft {
            extension_id,
            normalized_url,
            manifest_toml,
            #[cfg(test)]
            manifest_record,
        })
    }

    pub(crate) async fn put_hosted_mcp_descriptor<F>(
        fs: &F,
        owner: &UserId,
        input: RegisterHostedMcpDescriptorInput,
    ) -> Result<RegisteredHostedMcpDescriptorPut, ProductWorkflowError>
    where
        F: RootFilesystem + ?Sized,
    {
        let draft = Self::synthesize_hosted_mcp_manifest_toml(owner, &input)?;
        let prior_same_name = Self::find_same_owner_same_name_descriptor(
            fs,
            owner,
            draft.extension_id(),
            &input.name,
        )
        .await?;
        let existing_count = Self::descriptor_count_for_owner(fs, owner).await?;
        let existing_same_id = Self::descriptor_exists(fs, owner, draft.extension_id()).await?;
        if !existing_same_id
            && prior_same_name.is_none()
            && existing_count >= REGISTERED_OWNER_DESCRIPTOR_LIMIT
        {
            return Err(ProductWorkflowError::InvalidBindingRequest {
                reason: format!(
                    "registered extension limit exceeded: maximum {REGISTERED_OWNER_DESCRIPTOR_LIMIT} per owner"
                ),
            });
        }

        let manifest_path = Self::manifest_path(owner, draft.extension_id())?;
        fs.write_file(&manifest_path, draft.manifest_toml().as_bytes())
            .await
            .map_err(|error| ProductWorkflowError::Transient {
                reason: format!("failed to write registered extension manifest: {error}"),
            })?;

        Ok(RegisteredHostedMcpDescriptorPut {
            descriptor: draft.into_descriptor(),
            prior_same_name,
        })
    }

    pub(crate) async fn delete_descriptor<F>(
        fs: &F,
        owner: &UserId,
        extension_id: &ExtensionId,
    ) -> Result<(), ProductWorkflowError>
    where
        F: RootFilesystem + ?Sized,
    {
        let descriptor_root = Self::descriptor_root(owner, extension_id)?;
        match fs.delete(&descriptor_root).await {
            Ok(()) => Ok(()),
            Err(FilesystemError::NotFound { .. }) | Err(FilesystemError::MountNotFound { .. }) => {
                Ok(())
            }
            Err(error) => Err(ProductWorkflowError::Transient {
                reason: format!("failed to delete registered extension descriptor: {error}"),
            }),
        }
    }

    pub(crate) async fn find_same_owner_same_name_descriptor<F>(
        fs: &F,
        owner: &UserId,
        replacement_id: &ExtensionId,
        name: &str,
    ) -> Result<Option<RegisteredHostedMcpDescriptor>, ProductWorkflowError>
    where
        F: RootFilesystem + ?Sized,
    {
        let normalized_name = name.trim();
        for package in Self::list_for_owner(fs, owner).await? {
            let manifest = &package.package.manifest;
            if &manifest.id != replacement_id && manifest.name == normalized_name {
                let ExtensionRuntime::Mcp { url: Some(url), .. } = &manifest.runtime else {
                    continue;
                };
                #[cfg(test)]
                let normalized_url = normalized_hosted_mcp_url(url)?;
                #[cfg(not(test))]
                normalized_hosted_mcp_url(url)?;
                return Ok(Some(RegisteredHostedMcpDescriptor {
                    extension_id: manifest.id.clone(),
                    #[cfg(test)]
                    normalized_url,
                    #[cfg(test)]
                    manifest_toml: package.manifest_toml.clone(),
                    #[cfg(test)]
                    manifest_record: parse_registered_manifest(owner, package.manifest_toml, None)?,
                }));
            }
        }
        Ok(None)
    }

    /// One owner's registered packages, reusing the shared filesystem
    /// package parser tagged with `ManifestSource::UserRegistered`.
    pub(crate) async fn list_for_owner<F>(
        fs: &F,
        owner: &UserId,
    ) -> Result<Vec<AvailableExtensionPackage>, ProductWorkflowError>
    where
        F: RootFilesystem + ?Sized,
    {
        let root = Self::owner_root(owner)?;
        load_filesystem_packages(
            fs,
            &root,
            ManifestSource::UserRegistered {
                owner: owner.clone(),
            },
        )
        .await
    }

    /// Every owner's registered packages. Boot-time-only concern: an
    /// `ExtensionInstallation` record carries no owner field yet (plan risk
    /// 2), so restore's fallback must search across all owners. Never call
    /// this from the live search/install path, which must stay scoped to
    /// the calling owner via [`list_for_owner`].
    pub(crate) async fn list_all<F>(
        fs: &F,
    ) -> Result<Vec<AvailableExtensionPackage>, ProductWorkflowError>
    where
        F: RootFilesystem + ?Sized,
    {
        let root = Self::registered_root()?;
        let entries = match fs.list_dir(&root).await {
            Ok(entries) => entries,
            Err(FilesystemError::NotFound { .. }) | Err(FilesystemError::MountNotFound { .. }) => {
                return Ok(Vec::new());
            }
            Err(error) => {
                return Err(ProductWorkflowError::Transient {
                    reason: format!("failed to list registered extension owners: {error}"),
                });
            }
        };
        let mut packages = Vec::new();
        for entry in entries {
            if entry.file_type != FileType::Directory {
                continue;
            }
            let Ok(owner) = UserId::new(entry.name.clone()) else {
                continue;
            };
            packages.extend(Self::list_for_owner(fs, &owner).await?);
        }
        Ok(packages)
    }

    async fn descriptor_count_for_owner<F>(
        fs: &F,
        owner: &UserId,
    ) -> Result<usize, ProductWorkflowError>
    where
        F: RootFilesystem + ?Sized,
    {
        let root = Self::owner_root(owner)?;
        let entries = match fs.list_dir(&root).await {
            Ok(entries) => entries,
            Err(FilesystemError::NotFound { .. }) | Err(FilesystemError::MountNotFound { .. }) => {
                return Ok(0);
            }
            Err(error) => {
                return Err(ProductWorkflowError::Transient {
                    reason: format!("failed to list registered extension descriptors: {error}"),
                });
            }
        };
        Ok(entries
            .into_iter()
            .filter(|entry| entry.file_type == FileType::Directory)
            .filter(|entry| ExtensionId::new(entry.name.clone()).is_ok())
            .count())
    }

    async fn descriptor_exists<F>(
        fs: &F,
        owner: &UserId,
        extension_id: &ExtensionId,
    ) -> Result<bool, ProductWorkflowError>
    where
        F: RootFilesystem + ?Sized,
    {
        let root = Self::descriptor_root(owner, extension_id)?;
        match fs.stat(&root).await {
            Ok(stat) => Ok(stat.file_type == FileType::Directory),
            Err(FilesystemError::NotFound { .. }) | Err(FilesystemError::MountNotFound { .. }) => {
                Ok(false)
            }
            Err(error) => Err(ProductWorkflowError::Transient {
                reason: format!("failed to stat registered extension descriptor: {error}"),
            }),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredDiscoveredCapabilityIds {
    schema_version: String,
    owner: UserId,
    extension_id: ExtensionId,
    capability_ids: Vec<CapabilityId>,
}

fn canonical_discovered_capability_ids(
    extension_id: &ExtensionId,
    capability_ids: &[CapabilityId],
) -> Result<Vec<CapabilityId>, String> {
    let prefix = format!("{}.", extension_id.as_str());
    let mut canonical = BTreeSet::new();
    for capability_id in capability_ids {
        if !capability_id.as_str().starts_with(&prefix) {
            return Err(format!(
                "registered MCP capability does not belong to extension {}",
                extension_id.as_str()
            ));
        }
        canonical.insert(capability_id.clone());
    }
    if canonical.len() > DISCOVERED_CAPABILITY_IDS_LIMIT {
        return Err(format!(
            "registered MCP capability inventory limit exceeded: maximum {DISCOVERED_CAPABILITY_IDS_LIMIT}"
        ));
    }
    Ok(canonical.into_iter().collect())
}

fn invalid_stored_inventory(reason: String) -> ProductWorkflowError {
    ProductWorkflowError::Transient {
        reason: format!("registered MCP capability inventory is invalid: {reason}"),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RegisterHostedMcpDescriptorInput {
    pub(crate) name: String,
    pub(crate) url: String,
}

#[derive(Debug, Clone)]
pub(crate) struct RegisteredHostedMcpDescriptorDraft {
    extension_id: ExtensionId,
    normalized_url: String,
    manifest_toml: String,
    #[cfg(test)]
    manifest_record: ExtensionManifestRecord,
}

impl RegisteredHostedMcpDescriptorDraft {
    pub(crate) fn extension_id(&self) -> &ExtensionId {
        &self.extension_id
    }

    pub(crate) fn manifest_toml(&self) -> &str {
        &self.manifest_toml
    }

    pub(crate) fn normalized_url(&self) -> &str {
        &self.normalized_url
    }

    fn into_descriptor(self) -> RegisteredHostedMcpDescriptor {
        RegisteredHostedMcpDescriptor {
            extension_id: self.extension_id,
            #[cfg(test)]
            normalized_url: self.normalized_url,
            #[cfg(test)]
            manifest_toml: self.manifest_toml,
            #[cfg(test)]
            manifest_record: self.manifest_record,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct RegisteredHostedMcpDescriptor {
    pub(crate) extension_id: ExtensionId,
    #[cfg(test)]
    pub(crate) normalized_url: String,
    #[cfg(test)]
    pub(crate) manifest_toml: String,
    #[cfg(test)]
    pub(crate) manifest_record: ExtensionManifestRecord,
}

#[derive(Debug, Clone)]
pub(crate) struct RegisteredHostedMcpDescriptorPut {
    pub(crate) descriptor: RegisteredHostedMcpDescriptor,
    pub(crate) prior_same_name: Option<RegisteredHostedMcpDescriptor>,
}

#[derive(Debug, Serialize)]
struct RegisteredHostedMcpManifestDto {
    schema_version: String,
    id: String,
    name: String,
    version: String,
    description: String,
    trust: String,
    runtime: RegisteredHostedMcpRuntimeDto,
}

#[derive(Debug, Serialize)]
struct RegisteredHostedMcpRuntimeDto {
    kind: String,
    transport: String,
    url: String,
}

fn validate_registered_name(name: &str) -> Result<(), ProductWorkflowError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(ProductWorkflowError::InvalidBindingRequest {
            reason: "registered extension name must not be empty".to_string(),
        });
    }
    Ok(())
}

fn normalized_hosted_mcp_url(url: &str) -> Result<String, ProductWorkflowError> {
    HostedMcpEndpoint::parse(url).ok_or_else(|| ProductWorkflowError::InvalidBindingRequest {
        reason: "registered MCP URL must be an https URL without credentials, query, or fragment"
            .to_string(),
    })?;
    let parsed = url::Url::parse(url).map_err(map_binding_error)?;
    let host = parsed
        .host_str()
        .ok_or_else(|| ProductWorkflowError::InvalidBindingRequest {
            reason: "registered MCP URL must include a host".to_string(),
        })?
        .to_ascii_lowercase();
    let path = normalize_mcp_path(parsed.path());
    let port = parsed
        .port()
        .map(|port| format!(":{port}"))
        .unwrap_or_default();
    Ok(format!("https://{host}{port}{path}"))
}

fn normalize_mcp_path(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        "/".to_string()
    } else {
        trimmed.to_string()
    }
}

fn parse_registered_manifest(
    owner: &UserId,
    manifest_toml: String,
    manifest_hash: Option<ironclaw_extensions::ManifestHash>,
) -> Result<ExtensionManifestRecord, ProductWorkflowError> {
    let host_ports = ironclaw_host_runtime::default_host_port_catalog().map_err(|error| {
        ProductWorkflowError::InvalidBindingRequest {
            reason: format!("host port catalog rejected registered extension: {error}"),
        }
    })?;
    let contracts =
        ironclaw_host_runtime::default_host_api_contract_registry().map_err(|error| {
            ProductWorkflowError::InvalidBindingRequest {
                reason: format!(
                    "host API contract registry rejected registered extension: {error}"
                ),
            }
        })?;
    ExtensionManifestRecord::from_toml_with_contracts(
        manifest_toml,
        ManifestSource::UserRegistered {
            owner: owner.clone(),
        },
        &host_ports,
        manifest_hash,
        &contracts,
    )
    .map_err(map_binding_error)
}

fn not_found() -> ProductWorkflowError {
    ProductWorkflowError::InvalidBindingRequest {
        reason: "available extension was not found".to_string(),
    }
}

fn map_binding_error(error: impl std::fmt::Display) -> ProductWorkflowError {
    ProductWorkflowError::InvalidBindingRequest {
        reason: error.to_string(),
    }
}

/// The additional (owner-registered) search matches to overlay on top of the
/// shared catalog's own `catalog.search(query)` results. `owner: None` (no
/// caller identity — e.g. a boot-time/system caller) contributes no overlay:
/// registered packages are visible only to their own owner, never to an
/// unscoped caller.
pub(crate) async fn search_with_owner_overlay<F>(
    fs: &F,
    owner: Option<&UserId>,
    query: &str,
) -> Result<Vec<AvailableExtensionPackage>, ProductWorkflowError>
where
    F: RootFilesystem + ?Sized,
{
    let Some(owner) = owner else {
        return Ok(Vec::new());
    };
    let normalized_query = query.trim().to_ascii_lowercase();
    let packages = RegisteredExtensionStore::list_for_owner(fs, owner).await?;
    Ok(packages
        .into_iter()
        .filter(|package| !is_internal_extension_package_ref(&package.package_ref))
        .filter(|package| package_matches_search(package, &normalized_query))
        .collect())
}

/// Resolve one package by ref: shared first-party catalog first, then `owner`'s
/// registered set. `owner: None` is a caller with no owner scope (the boot-time
/// NEAR AI bootstrap installer) and can reach first-party packages only.
pub(crate) async fn resolve_with_owner_overlay<F>(
    catalog: &AvailableExtensionCatalog,
    fs: &F,
    owner: Option<&UserId>,
    package_ref: &LifecyclePackageRef,
) -> Result<AvailableExtensionPackage, ProductWorkflowError>
where
    F: RootFilesystem + ?Sized,
{
    if let Ok(available) = catalog.resolve(package_ref) {
        return Ok(available.clone());
    }
    let Some(owner) = owner else {
        return Err(not_found());
    };
    let owner_packages = RegisteredExtensionStore::list_for_owner(fs, owner).await?;
    owner_packages
        .into_iter()
        .find(|package| &package.package_ref == package_ref)
        .ok_or_else(not_found)
}

/// Boot-only restore fallback, reached on a `catalog.resolve()` miss during
/// `restore_extension_lifecycle_state`. Deliberately any-owner because
/// installations carry no owner field yet; never call it from a live
/// search/install path, which must stay scoped via
/// [`resolve_with_owner_overlay`].
pub(crate) async fn resolve_any_owner_for_restore<F>(
    fs: &F,
    package_ref: &LifecyclePackageRef,
) -> Result<AvailableExtensionPackage, ProductWorkflowError>
where
    F: RootFilesystem + ?Sized,
{
    let packages = RegisteredExtensionStore::list_all(fs).await?;
    packages
        .into_iter()
        .find(|package| &package.package_ref == package_ref)
        .ok_or_else(not_found)
}
