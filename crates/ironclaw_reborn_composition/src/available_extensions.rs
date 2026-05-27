use ironclaw_extensions::{
    CapabilityVisibility, ExtensionAssetPath, ExtensionManifest, ExtensionPackage,
    ExtensionRuntime, ManifestSource,
};
use ironclaw_filesystem::{FileType, FilesystemError, RootFilesystem};
use ironclaw_host_api::{CapabilityId, ExtensionId, VirtualPath, sha256_digest_token};
use ironclaw_product_workflow::{
    LifecycleExtensionSource, LifecycleExtensionSummary, LifecyclePackageKind, LifecyclePackageRef,
    ProductWorkflowError,
};

const GITHUB_MANIFEST: &str =
    include_str!("../../ironclaw_first_party_extensions/assets/github/manifest.toml");
const GITHUB_WASM_MODULE: &[u8] =
    include_bytes!("../../ironclaw_first_party_extensions/assets/github/wasm/github_tool.wasm");
const GOOGLE_CALENDAR_MANIFEST: &str =
    include_str!("../../ironclaw_first_party_extensions/assets/google-calendar/manifest.toml");
const GMAIL_MANIFEST: &str =
    include_str!("../../ironclaw_first_party_extensions/assets/gmail/manifest.toml");

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct AvailableExtensionAsset {
    pub(crate) path: String,
    pub(crate) content: AvailableExtensionAssetContent,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum AvailableExtensionAssetContent {
    Bytes(Vec<u8>),
    Filesystem(VirtualPath),
}

#[derive(Debug)]
pub(crate) struct AvailableExtensionPackage {
    pub(crate) package_ref: LifecyclePackageRef,
    pub(crate) manifest_toml: String,
    pub(crate) package: ExtensionPackage,
    pub(crate) assets: Vec<AvailableExtensionAsset>,
}

impl AvailableExtensionPackage {
    pub(crate) fn summary(&self) -> LifecycleExtensionSummary {
        let visible_read_only_capability_ids = visible_capability_ids(self)
            .map(|id| id.as_str().to_string())
            .collect::<Vec<_>>();
        LifecycleExtensionSummary {
            package_ref: self.package_ref.clone(),
            name: self.package.manifest.name.clone(),
            version: self.package.manifest.version.clone(),
            description: self.package.manifest.description.clone(),
            source: LifecycleExtensionSource::HostBundled,
            visible_read_only_capability_ids,
        }
    }
}

#[derive(Debug, Default)]
pub(crate) struct AvailableExtensionCatalog {
    packages: Vec<AvailableExtensionPackage>,
}

impl AvailableExtensionCatalog {
    pub(crate) fn from_packages(packages: Vec<AvailableExtensionPackage>) -> Self {
        Self { packages }
    }

    pub(crate) fn from_first_party_assets() -> Result<Self, ProductWorkflowError> {
        Ok(Self::from_packages(vec![
            github_package()?,
            google_calendar_package()?,
            gmail_package()?,
        ]))
    }

    pub(crate) fn extend(&mut self, other: Self) {
        for package in other.packages {
            if let Some(existing) = self
                .packages
                .iter_mut()
                .find(|existing| existing.package_ref == package.package_ref)
            {
                *existing = package;
            } else {
                self.packages.push(package);
            }
        }
    }

    pub(crate) async fn from_filesystem_root<F>(
        fs: &F,
        root: &VirtualPath,
    ) -> Result<Self, ProductWorkflowError>
    where
        F: RootFilesystem + ?Sized,
    {
        Ok(Self::from_packages(
            load_filesystem_packages(fs, root).await?,
        ))
    }

    pub(crate) fn search<'a>(
        &'a self,
        query: &str,
    ) -> impl Iterator<Item = &'a AvailableExtensionPackage> + 'a {
        let normalized_query = query.trim().to_ascii_lowercase();
        self.packages.iter().filter(move |package| {
            normalized_query.is_empty()
                || package
                    .package_ref
                    .id
                    .as_str()
                    .to_ascii_lowercase()
                    .contains(&normalized_query)
                || package
                    .package
                    .manifest
                    .name
                    .to_ascii_lowercase()
                    .contains(&normalized_query)
                || package
                    .package
                    .manifest
                    .description
                    .to_ascii_lowercase()
                    .contains(&normalized_query)
        })
    }

    pub(crate) fn resolve(
        &self,
        package_ref: &LifecyclePackageRef,
    ) -> Result<&AvailableExtensionPackage, ProductWorkflowError> {
        package_ref.require_kind(LifecyclePackageKind::Extension)?;
        self.packages
            .iter()
            .find(|package| &package.package_ref == package_ref)
            .ok_or_else(|| ProductWorkflowError::InvalidBindingRequest {
                reason: "available extension was not found".to_string(),
            })
    }
}

fn github_package() -> Result<AvailableExtensionPackage, ProductWorkflowError> {
    bundled_extension_package("github", "GitHub", GITHUB_MANIFEST, github_assets())
}

fn google_calendar_package() -> Result<AvailableExtensionPackage, ProductWorkflowError> {
    bundled_extension_package(
        "google-calendar",
        "Google Calendar",
        GOOGLE_CALENDAR_MANIFEST,
        google_calendar_assets(),
    )
}

fn gmail_package() -> Result<AvailableExtensionPackage, ProductWorkflowError> {
    bundled_extension_package("gmail", "Gmail", GMAIL_MANIFEST, gmail_assets())
}

pub(crate) fn google_calendar_manifest_digest() -> String {
    sha256_digest_token(GOOGLE_CALENDAR_MANIFEST.as_bytes())
}

pub(crate) fn gmail_manifest_digest() -> String {
    sha256_digest_token(GMAIL_MANIFEST.as_bytes())
}

fn bundled_extension_package(
    id: &str,
    label: &str,
    manifest_toml: &str,
    assets: Vec<AvailableExtensionAsset>,
) -> Result<AvailableExtensionPackage, ProductWorkflowError> {
    let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, id)?;
    let root = VirtualPath::new(format!("/system/extensions/{id}")).map_err(map_binding_error)?;
    let host_ports = ironclaw_host_runtime::default_host_port_catalog().map_err(|error| {
        ProductWorkflowError::InvalidBindingRequest {
            reason: format!("host port catalog rejected bundled {label} extension: {error}"),
        }
    })?;
    let contracts =
        ironclaw_host_runtime::default_host_api_contract_registry().map_err(|error| {
            ProductWorkflowError::InvalidBindingRequest {
                reason: format!("host API contracts rejected bundled {label} extension: {error}"),
            }
        })?;
    let manifest = ExtensionManifest::parse_with_optional_host_api_contracts(
        manifest_toml,
        ManifestSource::HostBundled,
        &host_ports,
        &contracts,
    )
    .map_err(|error| ProductWorkflowError::InvalidBindingRequest {
        reason: format!("bundled {label} extension manifest is invalid: {error}"),
    })?;
    let package =
        ExtensionPackage::from_manifest_toml(manifest, root, manifest_toml).map_err(|error| {
            ProductWorkflowError::InvalidBindingRequest {
                reason: format!("bundled {label} extension package is invalid: {error}"),
            }
        })?;
    Ok(AvailableExtensionPackage {
        package_ref,
        manifest_toml: manifest_toml.to_string(),
        package,
        assets,
    })
}

fn github_assets() -> Vec<AvailableExtensionAsset> {
    vec![
        bytes_asset("manifest.toml", GITHUB_MANIFEST.as_bytes()),
        bytes_asset(
            "schemas/github/search_issues.input.v1.json",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/github/schemas/github/search_issues.input.v1.json"
            ),
        ),
        bytes_asset(
            "schemas/github/search_issues.output.v1.json",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/github/schemas/github/search_issues.output.v1.json"
            ),
        ),
        bytes_asset(
            "schemas/github/get_issue.input.v1.json",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/github/schemas/github/get_issue.input.v1.json"
            ),
        ),
        bytes_asset(
            "schemas/github/get_issue.output.v1.json",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/github/schemas/github/get_issue.output.v1.json"
            ),
        ),
        bytes_asset(
            "schemas/github/comment_issue.input.v1.json",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/github/schemas/github/comment_issue.input.v1.json"
            ),
        ),
        bytes_asset(
            "schemas/github/comment_issue.output.v1.json",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/github/schemas/github/comment_issue.output.v1.json"
            ),
        ),
        bytes_asset(
            "prompts/github/search_issues.md",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/github/prompts/github/search_issues.md"
            ),
        ),
        bytes_asset(
            "prompts/github/get_issue.md",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/github/prompts/github/get_issue.md"
            ),
        ),
        bytes_asset(
            "prompts/github/comment_issue.md",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/github/prompts/github/comment_issue.md"
            ),
        ),
        bytes_asset("wasm/github_tool.wasm", GITHUB_WASM_MODULE),
    ]
}

fn google_calendar_assets() -> Vec<AvailableExtensionAsset> {
    vec![
        bytes_asset("manifest.toml", GOOGLE_CALENDAR_MANIFEST.as_bytes()),
        bytes_asset(
            "schemas/google-calendar/list_calendars.input.v1.json",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/google-calendar/schemas/google-calendar/list_calendars.input.v1.json"
            ),
        ),
        bytes_asset(
            "schemas/google-calendar/list_calendars.output.v1.json",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/google-calendar/schemas/google-calendar/list_calendars.output.v1.json"
            ),
        ),
        bytes_asset(
            "schemas/google-calendar/list_events.input.v1.json",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/google-calendar/schemas/google-calendar/list_events.input.v1.json"
            ),
        ),
        bytes_asset(
            "schemas/google-calendar/list_events.output.v1.json",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/google-calendar/schemas/google-calendar/list_events.output.v1.json"
            ),
        ),
        bytes_asset(
            "schemas/google-calendar/get_event.input.v1.json",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/google-calendar/schemas/google-calendar/get_event.input.v1.json"
            ),
        ),
        bytes_asset(
            "schemas/google-calendar/get_event.output.v1.json",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/google-calendar/schemas/google-calendar/get_event.output.v1.json"
            ),
        ),
        bytes_asset(
            "schemas/google-calendar/find_free_slots.input.v1.json",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/google-calendar/schemas/google-calendar/find_free_slots.input.v1.json"
            ),
        ),
        bytes_asset(
            "schemas/google-calendar/find_free_slots.output.v1.json",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/google-calendar/schemas/google-calendar/find_free_slots.output.v1.json"
            ),
        ),
        bytes_asset(
            "schemas/google-calendar/create_event.input.v1.json",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/google-calendar/schemas/google-calendar/create_event.input.v1.json"
            ),
        ),
        bytes_asset(
            "schemas/google-calendar/create_event.output.v1.json",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/google-calendar/schemas/google-calendar/create_event.output.v1.json"
            ),
        ),
        bytes_asset(
            "schemas/google-calendar/update_event.input.v1.json",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/google-calendar/schemas/google-calendar/update_event.input.v1.json"
            ),
        ),
        bytes_asset(
            "schemas/google-calendar/update_event.output.v1.json",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/google-calendar/schemas/google-calendar/update_event.output.v1.json"
            ),
        ),
        bytes_asset(
            "schemas/google-calendar/delete_event.input.v1.json",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/google-calendar/schemas/google-calendar/delete_event.input.v1.json"
            ),
        ),
        bytes_asset(
            "schemas/google-calendar/delete_event.output.v1.json",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/google-calendar/schemas/google-calendar/delete_event.output.v1.json"
            ),
        ),
        bytes_asset(
            "schemas/google-calendar/add_attendees.input.v1.json",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/google-calendar/schemas/google-calendar/add_attendees.input.v1.json"
            ),
        ),
        bytes_asset(
            "schemas/google-calendar/add_attendees.output.v1.json",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/google-calendar/schemas/google-calendar/add_attendees.output.v1.json"
            ),
        ),
        bytes_asset(
            "schemas/google-calendar/set_reminder.input.v1.json",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/google-calendar/schemas/google-calendar/set_reminder.input.v1.json"
            ),
        ),
        bytes_asset(
            "schemas/google-calendar/set_reminder.output.v1.json",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/google-calendar/schemas/google-calendar/set_reminder.output.v1.json"
            ),
        ),
        bytes_asset(
            "prompts/google-calendar/list_calendars.md",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/google-calendar/prompts/google-calendar/list_calendars.md"
            ),
        ),
        bytes_asset(
            "prompts/google-calendar/list_events.md",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/google-calendar/prompts/google-calendar/list_events.md"
            ),
        ),
        bytes_asset(
            "prompts/google-calendar/get_event.md",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/google-calendar/prompts/google-calendar/get_event.md"
            ),
        ),
        bytes_asset(
            "prompts/google-calendar/find_free_slots.md",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/google-calendar/prompts/google-calendar/find_free_slots.md"
            ),
        ),
        bytes_asset(
            "prompts/google-calendar/create_event.md",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/google-calendar/prompts/google-calendar/create_event.md"
            ),
        ),
        bytes_asset(
            "prompts/google-calendar/update_event.md",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/google-calendar/prompts/google-calendar/update_event.md"
            ),
        ),
        bytes_asset(
            "prompts/google-calendar/delete_event.md",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/google-calendar/prompts/google-calendar/delete_event.md"
            ),
        ),
        bytes_asset(
            "prompts/google-calendar/add_attendees.md",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/google-calendar/prompts/google-calendar/add_attendees.md"
            ),
        ),
        bytes_asset(
            "prompts/google-calendar/set_reminder.md",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/google-calendar/prompts/google-calendar/set_reminder.md"
            ),
        ),
    ]
}

fn gmail_assets() -> Vec<AvailableExtensionAsset> {
    vec![
        bytes_asset("manifest.toml", GMAIL_MANIFEST.as_bytes()),
        bytes_asset(
            "schemas/gmail/list_messages.input.v1.json",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/gmail/schemas/gmail/list_messages.input.v1.json"
            ),
        ),
        bytes_asset(
            "schemas/gmail/list_messages.output.v1.json",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/gmail/schemas/gmail/list_messages.output.v1.json"
            ),
        ),
        bytes_asset(
            "schemas/gmail/get_message.input.v1.json",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/gmail/schemas/gmail/get_message.input.v1.json"
            ),
        ),
        bytes_asset(
            "schemas/gmail/get_message.output.v1.json",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/gmail/schemas/gmail/get_message.output.v1.json"
            ),
        ),
        bytes_asset(
            "schemas/gmail/send_message.input.v1.json",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/gmail/schemas/gmail/send_message.input.v1.json"
            ),
        ),
        bytes_asset(
            "schemas/gmail/send_message.output.v1.json",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/gmail/schemas/gmail/send_message.output.v1.json"
            ),
        ),
        bytes_asset(
            "schemas/gmail/create_draft.input.v1.json",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/gmail/schemas/gmail/create_draft.input.v1.json"
            ),
        ),
        bytes_asset(
            "schemas/gmail/create_draft.output.v1.json",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/gmail/schemas/gmail/create_draft.output.v1.json"
            ),
        ),
        bytes_asset(
            "schemas/gmail/reply_to_message.input.v1.json",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/gmail/schemas/gmail/reply_to_message.input.v1.json"
            ),
        ),
        bytes_asset(
            "schemas/gmail/reply_to_message.output.v1.json",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/gmail/schemas/gmail/reply_to_message.output.v1.json"
            ),
        ),
        bytes_asset(
            "schemas/gmail/trash_message.input.v1.json",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/gmail/schemas/gmail/trash_message.input.v1.json"
            ),
        ),
        bytes_asset(
            "schemas/gmail/trash_message.output.v1.json",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/gmail/schemas/gmail/trash_message.output.v1.json"
            ),
        ),
        bytes_asset(
            "prompts/gmail/list_messages.md",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/gmail/prompts/gmail/list_messages.md"
            ),
        ),
        bytes_asset(
            "prompts/gmail/get_message.md",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/gmail/prompts/gmail/get_message.md"
            ),
        ),
        bytes_asset(
            "prompts/gmail/send_message.md",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/gmail/prompts/gmail/send_message.md"
            ),
        ),
        bytes_asset(
            "prompts/gmail/create_draft.md",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/gmail/prompts/gmail/create_draft.md"
            ),
        ),
        bytes_asset(
            "prompts/gmail/reply_to_message.md",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/gmail/prompts/gmail/reply_to_message.md"
            ),
        ),
        bytes_asset(
            "prompts/gmail/trash_message.md",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/gmail/prompts/gmail/trash_message.md"
            ),
        ),
    ]
}

fn bytes_asset(path: &str, bytes: &[u8]) -> AvailableExtensionAsset {
    AvailableExtensionAsset {
        path: path.to_string(),
        content: AvailableExtensionAssetContent::Bytes(bytes.to_vec()),
    }
}

pub(crate) async fn materialize_available_extension<F>(
    fs: &F,
    extension: &AvailableExtensionPackage,
) -> Result<(), ProductWorkflowError>
where
    F: RootFilesystem + ?Sized,
{
    let mut written_paths = Vec::new();
    for asset in &extension.assets {
        let path = extension_asset_path(&extension.package.id, &asset.path)?;
        let bytes = match &asset.content {
            AvailableExtensionAssetContent::Bytes(bytes) => bytes.clone(),
            AvailableExtensionAssetContent::Filesystem(source_path) => {
                match fs.read_file(source_path).await {
                    Ok(bytes) => bytes,
                    Err(error) => {
                        for written_path in written_paths.iter().rev() {
                            let _ = fs.delete(written_path).await;
                        }
                        return Err(ProductWorkflowError::Transient {
                            reason: format!(
                                "failed to read available extension asset {}: {error}",
                                asset.path
                            ),
                        });
                    }
                }
            }
        };
        if let Err(error) = fs.write_file(&path, &bytes).await {
            for written_path in written_paths.iter().rev() {
                let _ = fs.delete(written_path).await;
            }
            return Err(ProductWorkflowError::Transient {
                reason: format!(
                    "failed to materialize extension asset {}: {error}",
                    asset.path
                ),
            });
        }
        written_paths.push(path);
    }
    Ok(())
}

async fn load_filesystem_packages<F>(
    fs: &F,
    root: &VirtualPath,
) -> Result<Vec<AvailableExtensionPackage>, ProductWorkflowError>
where
    F: RootFilesystem + ?Sized,
{
    let mut entries = match fs.list_dir(root).await {
        Ok(entries) => entries,
        Err(FilesystemError::NotFound { .. }) | Err(FilesystemError::MountNotFound { .. }) => {
            return Ok(Vec::new());
        }
        Err(error) => {
            return Err(ProductWorkflowError::Transient {
                reason: format!("failed to list available extensions: {error}"),
            });
        }
    };
    entries.sort_by(|left, right| left.name.cmp(&right.name));

    let host_ports = ironclaw_host_runtime::default_host_port_catalog().map_err(|error| {
        ProductWorkflowError::InvalidBindingRequest {
            reason: format!("host port catalog rejected available extension: {error}"),
        }
    })?;
    let contracts =
        ironclaw_host_runtime::default_host_api_contract_registry().map_err(|error| {
            ProductWorkflowError::InvalidBindingRequest {
                reason: format!("host API contract registry rejected available extension: {error}"),
            }
        })?;

    let mut packages = Vec::new();
    for entry in entries {
        if entry.file_type != FileType::Directory {
            continue;
        }
        if ExtensionId::new(entry.name.clone()).is_err() {
            continue;
        }
        let manifest_path = VirtualPath::new(format!(
            "{}/manifest.toml",
            entry.path.as_str().trim_end_matches('/')
        ))
        .map_err(map_binding_error)?;
        let manifest_bytes = match fs.read_file(&manifest_path).await {
            Ok(bytes) => bytes,
            Err(FilesystemError::NotFound { .. }) | Err(FilesystemError::MountNotFound { .. }) => {
                continue;
            }
            Err(error) => {
                return Err(ProductWorkflowError::Transient {
                    reason: format!("failed to read available extension manifest: {error}"),
                });
            }
        };
        let manifest_toml = String::from_utf8(manifest_bytes).map_err(|error| {
            ProductWorkflowError::InvalidBindingRequest {
                reason: format!("available extension manifest is not UTF-8: {error}"),
            }
        })?;
        let manifest = ExtensionManifest::parse_with_optional_host_api_contracts(
            &manifest_toml,
            ManifestSource::HostBundled,
            &host_ports,
            &contracts,
        )
        .map_err(map_binding_error)?;
        let package = ExtensionPackage::from_manifest_toml(manifest, entry.path, &manifest_toml)
            .map_err(map_binding_error)?;
        let mut assets = vec![AvailableExtensionAsset {
            path: "manifest.toml".to_string(),
            content: AvailableExtensionAssetContent::Bytes(manifest_toml.as_bytes().to_vec()),
        }];
        if let ExtensionRuntime::Wasm { module } = &package.manifest.runtime {
            let module_path = module
                .resolve_under(&package.root)
                .map_err(map_binding_error)?;
            assets.push(AvailableExtensionAsset {
                path: module.as_str().to_string(),
                content: AvailableExtensionAssetContent::Filesystem(module_path),
            });
        }
        packages.push(AvailableExtensionPackage {
            package_ref: LifecyclePackageRef::new(
                LifecyclePackageKind::Extension,
                package.id.as_str(),
            )?,
            manifest_toml,
            package,
            assets,
        });
    }
    Ok(packages)
}

fn extension_asset_path(
    extension_id: &ExtensionId,
    asset_path: &str,
) -> Result<VirtualPath, ProductWorkflowError> {
    let root = VirtualPath::new(format!("/system/extensions/{}", extension_id.as_str()))
        .map_err(map_binding_error)?;
    ExtensionAssetPath::new(asset_path.to_string())
        .map_err(map_binding_error)?
        .resolve_under(&root)
        .map_err(map_binding_error)
}

fn map_binding_error(error: impl std::fmt::Display) -> ProductWorkflowError {
    ProductWorkflowError::InvalidBindingRequest {
        reason: error.to_string(),
    }
}

pub(crate) fn visible_capability_ids(
    extension: &AvailableExtensionPackage,
) -> impl Iterator<Item = &CapabilityId> {
    extension
        .package
        .manifest
        .capabilities
        .iter()
        .filter(|capability| capability.visibility == CapabilityVisibility::Model)
        .filter(|capability| !capability.effects.iter().any(|effect| effect.is_write()))
        .map(|capability| &capability.id)
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashSet,
        sync::{Arc, Mutex},
    };

    use async_trait::async_trait;
    use ironclaw_extensions::{ExtensionManifest, ManifestSource};
    use ironclaw_filesystem::{
        BackendCapabilities, DirEntry, FileStat, FilesystemError, FilesystemOperation,
        InMemoryBackend,
    };
    use ironclaw_host_api::{EffectKind, HostPortCatalog};

    use super::*;

    #[test]
    fn visible_capability_ids_excludes_write_effects() {
        let extension = test_extension_package();

        let visible = visible_capability_ids(&extension)
            .cloned()
            .collect::<Vec<_>>();

        assert_eq!(visible, vec![CapabilityId::new("fixture.search").unwrap()]);
        assert!(EffectKind::ExternalWrite.is_write());
        assert!(!EffectKind::Network.is_write());
    }

    #[test]
    fn bundled_gsuite_manifest_asset_refs_are_packaged() {
        let catalog = AvailableExtensionCatalog::from_first_party_assets().unwrap();

        for extension_id in ["google-calendar", "gmail"] {
            let package_ref =
                LifecyclePackageRef::new(LifecyclePackageKind::Extension, extension_id).unwrap();
            let package = catalog.resolve(&package_ref).unwrap();
            let assets = package
                .assets
                .iter()
                .map(|asset| asset.path.as_str())
                .collect::<HashSet<_>>();

            for capability in &package.package.manifest.capabilities {
                assert!(
                    assets.contains(capability.input_schema_ref.as_str()),
                    "{extension_id} capability {} missing input schema asset {}",
                    capability.id,
                    capability.input_schema_ref.as_str()
                );
                assert!(
                    assets.contains(capability.output_schema_ref.as_str()),
                    "{extension_id} capability {} missing output schema asset {}",
                    capability.id,
                    capability.output_schema_ref.as_str()
                );
                if let Some(prompt_doc_ref) = &capability.prompt_doc_ref {
                    assert!(
                        assets.contains(prompt_doc_ref.as_str()),
                        "{extension_id} capability {} missing prompt doc asset {}",
                        capability.id,
                        prompt_doc_ref.as_str()
                    );
                }
            }
        }
    }

    #[test]
    fn catalog_extend_replaces_duplicate_package_refs() {
        let stale = test_extension_package_with_wasm_bytes(b"stale");
        let bundled = test_extension_package_with_wasm_bytes(b"bundled");
        let mut catalog = AvailableExtensionCatalog::from_packages(vec![stale]);
        catalog.extend(AvailableExtensionCatalog::from_packages(vec![bundled]));

        let package_ref =
            LifecyclePackageRef::new(LifecyclePackageKind::Extension, "fixture").unwrap();
        let package = catalog.resolve(&package_ref).unwrap();
        let wasm = package
            .assets
            .iter()
            .find(|asset| asset.path == "wasm/fixture.wasm")
            .expect("wasm asset");

        assert_eq!(
            wasm.content,
            AvailableExtensionAssetContent::Bytes(b"bundled".to_vec())
        );
        assert_eq!(catalog.search("fixture").count(), 1);
    }

    #[tokio::test]
    async fn materialize_fails_on_filesystem_error_and_rolls_back_written_assets() {
        let fs = FailingWriteFilesystem::default();
        let extension = test_extension_package();

        let error = materialize_available_extension(&fs, &extension)
            .await
            .expect_err("second write fails");

        assert!(matches!(error, ProductWorkflowError::Transient { .. }));
        let state = fs.state.lock().unwrap();
        assert_eq!(
            state.writes,
            vec![
                "/system/extensions/fixture/manifest.toml".to_string(),
                "/system/extensions/fixture/wasm/fixture.wasm".to_string()
            ]
        );
        assert_eq!(
            state.deletes,
            vec!["/system/extensions/fixture/manifest.toml".to_string()]
        );
    }

    #[tokio::test]
    async fn filesystem_catalog_loads_manifest_and_runtime_assets() {
        let fs = InMemoryBackend::default();
        let extension = test_extension_package();
        for asset in &extension.assets {
            let path = extension_asset_path(&extension.package.id, &asset.path).unwrap();
            let AvailableExtensionAssetContent::Bytes(bytes) = &asset.content else {
                panic!("test fixture assets are byte-backed");
            };
            fs.write_file(&path, bytes).await.unwrap();
        }

        let catalog = AvailableExtensionCatalog::from_filesystem_root(
            &fs,
            &VirtualPath::new("/system/extensions").unwrap(),
        )
        .await
        .unwrap();
        let results = catalog.search("fixture").collect::<Vec<_>>();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].package_ref, extension.package_ref);
        assert_eq!(
            results[0]
                .assets
                .iter()
                .map(|asset| asset.path.as_str())
                .collect::<Vec<_>>(),
            vec!["manifest.toml", "wasm/fixture.wasm"]
        );
    }

    #[tokio::test]
    async fn filesystem_catalog_skips_extension_dirs_without_manifest() {
        let fs = InMemoryBackend::default();
        fs.write_file(
            &VirtualPath::new("/system/extensions/incomplete/cache/leftover").unwrap(),
            b"stale",
        )
        .await
        .unwrap();

        let catalog = AvailableExtensionCatalog::from_filesystem_root(
            &fs,
            &VirtualPath::new("/system/extensions").unwrap(),
        )
        .await
        .unwrap();

        assert_eq!(catalog.search("").count(), 0);
    }

    #[derive(Default)]
    struct FailingWriteFilesystem {
        state: Arc<Mutex<FailingWriteState>>,
    }

    #[derive(Default)]
    struct FailingWriteState {
        writes: Vec<String>,
        deletes: Vec<String>,
    }

    #[async_trait]
    impl RootFilesystem for FailingWriteFilesystem {
        fn capabilities(&self) -> BackendCapabilities {
            BackendCapabilities::default()
        }

        async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
            Err(FilesystemError::Unsupported {
                path: path.clone(),
                operation: FilesystemOperation::ListDir,
            })
        }

        async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
            Err(FilesystemError::NotFound {
                path: path.clone(),
                operation: FilesystemOperation::Stat,
            })
        }

        async fn write_file(
            &self,
            path: &VirtualPath,
            _bytes: &[u8],
        ) -> Result<(), FilesystemError> {
            self.state
                .lock()
                .unwrap()
                .writes
                .push(path.as_str().to_string());
            if path.as_str().ends_with("fixture.wasm") {
                return Err(FilesystemError::Backend {
                    path: path.clone(),
                    operation: FilesystemOperation::WriteFile,
                    reason: "write rejected".to_string(),
                });
            }
            Ok(())
        }

        async fn delete(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
            self.state
                .lock()
                .unwrap()
                .deletes
                .push(path.as_str().to_string());
            Ok(())
        }
    }

    fn test_extension_package() -> AvailableExtensionPackage {
        test_extension_package_with_wasm_bytes(b"wasm")
    }

    fn test_extension_package_with_wasm_bytes(wasm_bytes: &[u8]) -> AvailableExtensionPackage {
        static MANIFEST: &str = r#"
schema_version = "reborn.extension_manifest.v2"
id = "fixture"
name = "Fixture"
version = "0.1.0"
description = "fixture extension"
trust = "third_party"

[runtime]
kind = "wasm"
module = "wasm/fixture.wasm"

[[capabilities]]
id = "fixture.search"
description = "Search"
effects = ["network"]
default_permission = "ask"
visibility = "model"
input_schema_ref = "schemas/search.input.json"
output_schema_ref = "schemas/search.output.json"

[[capabilities]]
id = "fixture.write"
description = "Write"
effects = ["external_write"]
default_permission = "ask"
visibility = "model"
input_schema_ref = "schemas/write.input.json"
output_schema_ref = "schemas/write.output.json"
"#;
        let manifest = ExtensionManifest::parse(
            MANIFEST,
            ManifestSource::HostBundled,
            &HostPortCatalog::empty(),
        )
        .expect("manifest");
        let package = ExtensionPackage::from_manifest(
            manifest,
            VirtualPath::new("/system/extensions/fixture").unwrap(),
        )
        .expect("package");
        AvailableExtensionPackage {
            package_ref: LifecyclePackageRef::new(LifecyclePackageKind::Extension, "fixture")
                .unwrap(),
            manifest_toml: MANIFEST.to_string(),
            package,
            assets: vec![
                AvailableExtensionAsset {
                    path: "manifest.toml".to_string(),
                    content: AvailableExtensionAssetContent::Bytes(MANIFEST.as_bytes().to_vec()),
                },
                AvailableExtensionAsset {
                    path: "wasm/fixture.wasm".to_string(),
                    content: AvailableExtensionAssetContent::Bytes(wasm_bytes.to_vec()),
                },
            ],
        }
    }
}
