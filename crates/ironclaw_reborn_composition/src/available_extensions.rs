use std::sync::Arc;

use ironclaw_extensions::{CapabilityVisibility, ExtensionAssetPath, ExtensionPackage};
use ironclaw_filesystem::RootFilesystem;
use ironclaw_host_api::{CapabilityId, EffectKind, ExtensionId, VirtualPath};
use ironclaw_product_workflow::{LifecyclePackageKind, LifecyclePackageRef, ProductWorkflowError};
use serde_json::{Value, json};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AvailableExtensionAsset {
    pub(crate) path: &'static str,
    pub(crate) bytes: &'static [u8],
}

#[derive(Debug, Clone)]
pub(crate) struct AvailableExtensionPackage {
    pub(crate) package_ref: LifecyclePackageRef,
    pub(crate) manifest_toml: &'static str,
    pub(crate) package: ExtensionPackage,
    pub(crate) assets: Vec<AvailableExtensionAsset>,
}

impl AvailableExtensionPackage {
    pub(crate) fn summary_json(&self) -> Value {
        let visible_read_only_capability_ids = self
            .package
            .manifest
            .capabilities
            .iter()
            .filter(|capability| capability.visibility == CapabilityVisibility::Model)
            .filter(|capability| !capability.effects.iter().any(is_write_effect))
            .map(|capability| capability.id.as_str().to_string())
            .collect::<Vec<_>>();
        json!({
            "package_ref": {
                "kind": lifecycle_kind_str(self.package_ref.kind),
                "id": self.package_ref.id.as_str(),
            },
            "name": self.package.manifest.name,
            "version": self.package.manifest.version,
            "description": self.package.manifest.description,
            "source": "host_bundled",
            "visible_read_only_capability_ids": visible_read_only_capability_ids,
        })
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct AvailableExtensionCatalog {
    sources: Vec<Arc<dyn AvailableExtensionSource>>,
}

impl AvailableExtensionCatalog {
    pub(crate) fn empty() -> Self {
        Self::default()
    }

    #[cfg(test)]
    pub(crate) fn from_packages(packages: Vec<AvailableExtensionPackage>) -> Self {
        Self {
            sources: vec![Arc::new(StaticAvailableExtensionSource { packages })],
        }
    }

    pub(crate) fn search(
        &self,
        query: &str,
    ) -> Result<Vec<AvailableExtensionPackage>, ProductWorkflowError> {
        let mut results = Vec::new();
        for source in &self.sources {
            results.extend(source.search(query)?);
        }
        Ok(results)
    }

    pub(crate) fn resolve(
        &self,
        package_ref: &LifecyclePackageRef,
    ) -> Result<AvailableExtensionPackage, ProductWorkflowError> {
        package_ref.require_kind(LifecyclePackageKind::Extension)?;
        for source in &self.sources {
            if let Some(package) = source.resolve(package_ref)? {
                return Ok(package);
            }
        }
        Err(ProductWorkflowError::InvalidBindingRequest {
            reason: "available extension was not found".to_string(),
        })
    }
}

trait AvailableExtensionSource: Send + Sync + std::fmt::Debug {
    fn search(&self, query: &str) -> Result<Vec<AvailableExtensionPackage>, ProductWorkflowError>;

    fn resolve(
        &self,
        package_ref: &LifecyclePackageRef,
    ) -> Result<Option<AvailableExtensionPackage>, ProductWorkflowError>;
}

#[cfg(test)]
#[derive(Debug)]
struct StaticAvailableExtensionSource {
    packages: Vec<AvailableExtensionPackage>,
}

#[cfg(test)]
impl AvailableExtensionSource for StaticAvailableExtensionSource {
    fn search(&self, query: &str) -> Result<Vec<AvailableExtensionPackage>, ProductWorkflowError> {
        let normalized_query = query.trim().to_ascii_lowercase();
        Ok(self
            .packages
            .iter()
            .filter(|package| {
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
            .cloned()
            .collect())
    }

    fn resolve(
        &self,
        package_ref: &LifecyclePackageRef,
    ) -> Result<Option<AvailableExtensionPackage>, ProductWorkflowError> {
        Ok(self
            .packages
            .iter()
            .find(|package| &package.package_ref == package_ref)
            .cloned())
    }
}

fn lifecycle_kind_str(kind: LifecyclePackageKind) -> &'static str {
    match kind {
        LifecyclePackageKind::Extension => "extension",
        LifecyclePackageKind::Skill => "skill",
        LifecyclePackageKind::Mcp => "mcp",
        LifecyclePackageKind::Wasm => "wasm",
    }
}

pub(crate) async fn materialize_available_extension<F>(
    fs: &F,
    extension: &AvailableExtensionPackage,
) -> Result<(), ProductWorkflowError>
where
    F: RootFilesystem,
{
    for asset in &extension.assets {
        let path = extension_asset_path(&extension.package.id, asset.path)?;
        fs.write_file(&path, asset.bytes).await.map_err(|error| {
            ProductWorkflowError::Transient {
                reason: format!(
                    "failed to materialize extension asset {}: {error}",
                    asset.path
                ),
            }
        })?;
    }
    Ok(())
}

fn extension_asset_path(
    extension_id: &ExtensionId,
    asset_path: &str,
) -> Result<VirtualPath, ProductWorkflowError> {
    let root = VirtualPath::new(format!("/system/extensions/{}", extension_id.as_str())).map_err(
        |error| ProductWorkflowError::InvalidBindingRequest {
            reason: error.to_string(),
        },
    )?;
    ExtensionAssetPath::new(asset_path.to_string())
        .map_err(|error| ProductWorkflowError::InvalidBindingRequest {
            reason: error.to_string(),
        })?
        .resolve_under(&root)
        .map_err(|error| ProductWorkflowError::InvalidBindingRequest {
            reason: error.to_string(),
        })
}

fn is_write_effect(effect: &EffectKind) -> bool {
    matches!(
        effect,
        EffectKind::WriteFilesystem
            | EffectKind::DeleteFilesystem
            | EffectKind::ExecuteCode
            | EffectKind::SpawnProcess
            | EffectKind::ModifyExtension
            | EffectKind::ModifyApproval
            | EffectKind::ModifyBudget
            | EffectKind::ExternalWrite
            | EffectKind::Financial
    )
}

pub(crate) fn visible_capability_ids(extension: &AvailableExtensionPackage) -> Vec<CapabilityId> {
    extension
        .package
        .manifest
        .capabilities
        .iter()
        .filter(|capability| capability.visibility == CapabilityVisibility::Model)
        .map(|capability| capability.id.clone())
        .collect()
}
