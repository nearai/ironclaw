use std::sync::Arc;

use ironclaw_extensions::{CapabilityVisibility, ExtensionAssetPath, ExtensionPackage};
use ironclaw_filesystem::RootFilesystem;
use ironclaw_host_api::{CapabilityId, ExtensionId, VirtualPath};
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
            .filter(|capability| !capability.effects.iter().any(|effect| effect.is_write()))
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
    let mut written_paths = Vec::new();
    for asset in &extension.assets {
        let path = extension_asset_path(&extension.package.id, asset.path)?;
        if let Err(error) = fs.write_file(&path, asset.bytes).await {
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

pub(crate) fn visible_capability_ids(extension: &AvailableExtensionPackage) -> Vec<CapabilityId> {
    extension
        .package
        .manifest
        .capabilities
        .iter()
        .filter(|capability| capability.visibility == CapabilityVisibility::Model)
        .filter(|capability| !capability.effects.iter().any(|effect| effect.is_write()))
        .map(|capability| capability.id.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use ironclaw_extensions::{ExtensionManifest, ManifestSource};
    use ironclaw_filesystem::{
        BackendCapabilities, DirEntry, FileStat, FilesystemError, FilesystemOperation,
    };
    use ironclaw_host_api::{EffectKind, HostPortCatalog};

    use super::*;

    #[test]
    fn visible_capability_ids_excludes_write_effects() {
        let extension = test_extension_package();

        let visible = visible_capability_ids(&extension);

        assert_eq!(visible, vec![CapabilityId::new("fixture.search").unwrap()]);
        assert!(EffectKind::ExternalWrite.is_write());
        assert!(!EffectKind::Network.is_write());
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
            manifest_toml: MANIFEST,
            package,
            assets: vec![
                AvailableExtensionAsset {
                    path: "manifest.toml",
                    bytes: MANIFEST.as_bytes(),
                },
                AvailableExtensionAsset {
                    path: "wasm/fixture.wasm",
                    bytes: b"wasm",
                },
            ],
        }
    }
}
