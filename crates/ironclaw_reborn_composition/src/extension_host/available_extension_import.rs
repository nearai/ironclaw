use std::sync::Arc;

use ironclaw_extensions::{
    ExtensionAssetPath, ExtensionManifestRecord, ExtensionPackage, ExtensionRuntimeV2,
    ManifestSource,
};
use ironclaw_filesystem::{FileType, FilesystemError, RootFilesystem};
use ironclaw_host_api::{ExtensionId, RuntimeKind, VirtualPath};
use ironclaw_product::{LifecyclePackageKind, LifecyclePackageRef, ProductWorkflowError};

use crate::extension_host::host_api_contracts::product_extension_host_api_contract_registry;

use super::available_extensions::{
    AvailableExtensionAsset, AvailableExtensionAssetContent, AvailableExtensionPackage,
    bytes_asset, map_binding_error, reserved_host_bundled_extension_id,
    surface_kinds_from_manifest_record,
};
use super::extension_bundle::{
    MAX_EXTENSION_BUNDLE_FILES, MAX_EXTENSION_BUNDLE_UNCOMPRESSED_BYTES,
};

/// Write a catalog package's inline assets to its stable extension root. The
/// same path is used by restore and upload import, so catalog entries remain
/// self-contained after a remove deletes the materialized directory.
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
        let AvailableExtensionAssetContent::Bytes(bytes) = &asset.content;
        let bytes = bytes.clone();
        if existing_asset_matches(fs, &path, &bytes).await {
            continue;
        }
        if let Err(error) = fs.write_file(&path, &bytes).await {
            for written_path in written_paths.iter().rev() {
                if let Err(error) = fs.delete(written_path).await {
                    tracing::debug!(
                        ?error,
                        "best-effort extension asset rollback cleanup failed"
                    );
                }
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

async fn existing_asset_matches<F>(fs: &F, path: &VirtualPath, bytes: &[u8]) -> bool
where
    F: RootFilesystem + ?Sized,
{
    match fs.read_file(path).await {
        Ok(existing) => existing == bytes,
        Err(FilesystemError::NotFound { .. }) | Err(FilesystemError::MountNotFound { .. }) => false,
        Err(_) => false,
    }
}

pub(crate) fn extension_asset_path(
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

/// Read every file under `root` into inline bytes (paths relative to `root`),
/// so filesystem discovery produces the same self-contained package shape as
/// a fresh import. Apply the upload limits here too: restart discovery must
/// not silently admit an extension directory that the upload boundary rejects.
pub(crate) async fn inline_extension_dir_assets<F>(
    fs: &F,
    root: &VirtualPath,
) -> Result<Vec<AvailableExtensionAsset>, ProductWorkflowError>
where
    F: RootFilesystem + ?Sized,
{
    let root_prefix = format!("{}/", root.as_str().trim_end_matches('/'));
    let mut assets = Vec::new();
    let mut total_bytes = 0usize;
    let mut pending = vec![root.clone()];
    while let Some(dir) = pending.pop() {
        let entries = fs
            .list_dir(&dir)
            .await
            .map_err(|error| ProductWorkflowError::Transient {
                reason: format!("failed to list available extension assets: {error}"),
            })?;
        for child in entries {
            if child.file_type == FileType::Directory {
                pending.push(child.path);
                continue;
            }
            if assets.len() >= MAX_EXTENSION_BUNDLE_FILES {
                return Err(map_binding_error(format!(
                    "extension at {} has too many files (limit {})",
                    root.as_str(),
                    MAX_EXTENSION_BUNDLE_FILES
                )));
            }
            let bytes = fs.read_file(&child.path).await.map_err(|error| {
                ProductWorkflowError::Transient {
                    reason: format!(
                        "failed to read available extension asset {}: {error}",
                        child.path.as_str()
                    ),
                }
            })?;
            total_bytes = total_bytes.saturating_add(bytes.len());
            if total_bytes > MAX_EXTENSION_BUNDLE_UNCOMPRESSED_BYTES {
                return Err(map_binding_error(format!(
                    "extension at {} exceeds the {}-byte asset limit",
                    root.as_str(),
                    MAX_EXTENSION_BUNDLE_UNCOMPRESSED_BYTES
                )));
            }
            let Some(relative) = child.path.as_str().strip_prefix(&root_prefix) else {
                return Err(map_binding_error(format!(
                    "available extension asset {} is outside expected root prefix {}",
                    child.path.as_str(),
                    root_prefix
                )));
            };
            assets.push(AvailableExtensionAsset {
                path: relative.to_string(),
                content: AvailableExtensionAssetContent::Bytes(bytes),
            });
        }
    }
    Ok(assets)
}

/// Build an [`AvailableExtensionPackage`] from an extracted upload. Manifest
/// validation, runtime restrictions, declared-asset completeness, and WASI
/// component validation all belong to this import boundary; lifecycle only
/// coordinates the bounded decode and subsequent catalog/filesystem writes.
pub(crate) fn imported_extension_package(
    files: Vec<(String, Vec<u8>)>,
    reserved_bundled_ids: &[String],
) -> Result<AvailableExtensionPackage, ProductWorkflowError> {
    let manifest_toml = files
        .iter()
        .find(|(path, _)| path == "manifest.toml")
        .ok_or_else(|| map_binding_error("imported bundle is missing manifest.toml at its root"))
        .and_then(|(_, bytes)| {
            String::from_utf8(bytes.clone()).map_err(|error| {
                map_binding_error(format!("imported manifest.toml is not UTF-8: {error}"))
            })
        })?;
    let host_ports = ironclaw_host_runtime::default_host_port_catalog().map_err(|error| {
        ProductWorkflowError::InvalidBindingRequest {
            reason: format!("host port catalog rejected imported extension: {error}"),
        }
    })?;
    let contracts = product_extension_host_api_contract_registry().map_err(|error| {
        ProductWorkflowError::InvalidBindingRequest {
            reason: format!("host API contract registry rejected imported extension: {error}"),
        }
    })?;
    // Uploads are always validated as InstalledLocal. Only binary-compiled
    // packages may claim the HostBundled trust/runtime tier.
    let record = ExtensionManifestRecord::from_toml(
        manifest_toml,
        ManifestSource::InstalledLocal,
        &host_ports,
        None,
        &contracts,
    )
    .map_err(map_binding_error)?;
    let runtime_kind = record.manifest().runtime.kind();
    if runtime_kind != RuntimeKind::Wasm {
        return Err(map_binding_error(format!(
            "imported tool bundles must declare a wasm runtime; got `{runtime_kind:?}`"
        )));
    }
    let extension_id = record.manifest().id.clone();
    if reserved_host_bundled_extension_id(&extension_id, reserved_bundled_ids) {
        return Err(map_binding_error(format!(
            "extension id `{}` is reserved for host-bundled extensions and cannot be imported",
            extension_id.as_str()
        )));
    }
    let id = extension_id.as_str();
    let root = VirtualPath::new(format!("/system/extensions/{id}")).map_err(map_binding_error)?;
    let surface_kinds = surface_kinds_from_manifest_record(&record, id)?;
    let manifest = record
        .manifest()
        .clone()
        .try_into()
        .map_err(map_binding_error)?;
    let package = ExtensionPackage::from_manifest_toml(manifest, root, record.raw_toml())
        .map_err(map_binding_error)?;

    let bundled_paths: std::collections::HashSet<&str> =
        files.iter().map(|(path, _)| path.as_str()).collect();
    for declared in manifest_declared_asset_paths(record.manifest()) {
        if !bundled_paths.contains(declared.as_str()) {
            return Err(map_binding_error(format!(
                "imported bundle is missing manifest-declared asset `{declared}`"
            )));
        }
    }
    if let ExtensionRuntimeV2::Wasm { module } = &record.manifest().runtime {
        let module_bytes = files
            .iter()
            .find(|(path, _)| path.as_str() == module.as_str())
            .map(|(_, bytes)| bytes.as_slice())
            .unwrap_or_default();
        let is_component = module_bytes.len() >= 8
            && module_bytes[..4] == *b"\0asm"
            && module_bytes[6..8] == [0x01, 0x00];
        if !is_component {
            return Err(map_binding_error(format!(
                "imported wasm module `{module}` is not a WASI component; the runtime cannot \
                 load core modules — build with `--target wasm32-wasip2` (or componentize a \
                 wasip1 module with `wasm-tools component new`)"
            )));
        }
    }
    let assets = files
        .into_iter()
        .map(|(path, bytes)| bytes_asset(&path, &bytes))
        .collect();
    Ok(AvailableExtensionPackage {
        package_ref: LifecyclePackageRef::new(
            LifecyclePackageKind::Extension,
            package.id.as_str(),
        )?,
        manifest_toml: record.raw_toml().to_string(),
        resolved_manifest: Arc::new(record.resolved().clone()),
        source: ManifestSource::InstalledLocal,
        package,
        cleanup_requirements: Vec::new(),
        surface_kinds,
        channel_directions: None,
        channel_presentation: None,
        assets,
        onboarding_override: None,
        oauth_setup_override: None,
        search_aliases: Vec::new(),
    })
}

fn manifest_declared_asset_paths(
    manifest: &ironclaw_extensions::ExtensionManifestV2,
) -> Vec<String> {
    let mut declared = Vec::new();
    if let ExtensionRuntimeV2::Wasm { module } = &manifest.runtime {
        declared.push(module.clone());
    }
    for capability in &manifest.capabilities {
        declared.push(capability.input_schema_ref.as_str().to_string());
        if let Some(output_schema_ref) = &capability.output_schema_ref {
            declared.push(output_schema_ref.as_str().to_string());
        }
        if let Some(prompt_doc_ref) = &capability.prompt_doc_ref {
            declared.push(prompt_doc_ref.as_str().to_string());
        }
    }
    declared
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use ironclaw_extensions::ManifestSource;
    use ironclaw_filesystem::{DirEntry, FileStat, FilesystemOperation, InMemoryBackend};
    use ironclaw_host_api::RuntimeKind;

    use crate::extension_host::available_extensions::{
        AvailableExtensionAssetContent, AvailableExtensionCatalog,
    };

    #[tokio::test]
    async fn filesystem_catalog_loads_manifest_and_runtime_assets() {
        let fs = InMemoryBackend::default();
        const MANIFEST: &str = r#"
schema_version = "reborn.extension_manifest.v2"
id = "fixture"
name = "Fixture"
version = "0.1.0"
description = "fixture extension"
trust = "third_party"

[runtime]
kind = "wasm"
module = "wasm/fixture.wasm"

[[host_api]]
id = "ironclaw.capability_provider/v1"
section = "capability_provider.tools"

[capability_provider.tools]

[[capability_provider.tools.capabilities]]
id = "fixture.search"
description = "Search"
effects = ["network"]
default_permission = "ask"
visibility = "model"
input_schema_ref = "schemas/search.input.json"
output_schema_ref = "schemas/search.output.json"
"#;
        fs.write_file(
            &VirtualPath::new("/system/extensions/fixture/manifest.toml").unwrap(),
            MANIFEST.as_bytes(),
        )
        .await
        .unwrap();
        fs.write_file(
            &VirtualPath::new("/system/extensions/fixture/wasm/fixture.wasm").unwrap(),
            b"wasm",
        )
        .await
        .unwrap();
        fs.write_file(
            &VirtualPath::new("/system/extensions/fixture/schemas/search.input.json").unwrap(),
            b"{}",
        )
        .await
        .unwrap();

        let catalog = AvailableExtensionCatalog::from_filesystem_root(
            &fs,
            &VirtualPath::new("/system/extensions").unwrap(),
            &[],
        )
        .await
        .unwrap();
        let results = catalog.search("fixture").collect::<Vec<_>>();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].source, ManifestSource::InstalledLocal);
        let mut asset_paths = results[0]
            .assets
            .iter()
            .map(|asset| asset.path.as_str())
            .collect::<Vec<_>>();
        asset_paths.sort_unstable();
        assert_eq!(
            asset_paths,
            vec![
                "manifest.toml",
                "schemas/search.input.json",
                "wasm/fixture.wasm"
            ]
        );
        assert!(
            results[0]
                .assets
                .iter()
                .all(|asset| matches!(asset.content, AvailableExtensionAssetContent::Bytes(_)))
        );

        fs.delete(&VirtualPath::new("/system/extensions/fixture").unwrap())
            .await
            .unwrap();
        materialize_available_extension(&fs, &results[0])
            .await
            .expect("reinstall after remove must re-materialize from catalog bytes");
        assert!(
            fs.read_file(
                &VirtualPath::new("/system/extensions/fixture/wasm/fixture.wasm").unwrap()
            )
            .await
            .is_ok()
        );
        assert!(
            fs.read_file(
                &VirtualPath::new("/system/extensions/fixture/schemas/search.input.json").unwrap()
            )
            .await
            .is_ok()
        );
    }

    #[tokio::test]
    async fn filesystem_asset_catalog_rejects_paths_outside_expected_root() {
        let root = VirtualPath::new("/system/extensions/fixture").unwrap();
        let error = inline_extension_dir_assets(&MismatchedAssetPathFilesystem, &root)
            .await
            .expect_err("asset paths outside the extension root must fail discovery");
        let ProductWorkflowError::InvalidBindingRequest { reason } = error else {
            panic!("expected invalid binding request, got {error:?}");
        };
        assert!(reason.contains("/system/extensions/other/asset.txt"));
        assert!(reason.contains("/system/extensions/fixture/"));
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
            &[],
        )
        .await
        .unwrap();
        assert_eq!(catalog.search("").count(), 0);
    }

    #[tokio::test]
    async fn filesystem_catalog_skips_reserved_host_bundled_extension_ids() {
        let fs = InMemoryBackend::default();
        for id in ["gmail"] {
            fs.write_file(
                &VirtualPath::new(format!("/system/extensions/{id}/manifest.toml")).unwrap(),
                b"not parsed because the id is host-bundled",
            )
            .await
            .unwrap();
        }
        let catalog = AvailableExtensionCatalog::from_filesystem_root(
            &fs,
            &VirtualPath::new("/system/extensions").unwrap(),
            &[],
        )
        .await
        .unwrap();
        assert_eq!(catalog.search("").count(), 0);
    }

    #[tokio::test]
    async fn filesystem_catalog_skips_manifest_with_forbidden_trust_instead_of_aborting() {
        // #5966: pre-#5499 installs materialized bundled first-party manifests
        // (trust = "first_party_requested") verbatim onto the persistent
        // volume. Boot rescan stamps filesystem manifests `InstalledLocal`,
        // which forbids that trust — one stale manifest must be skipped, not
        // abort the whole catalog load (crash-looping the deployment); the
        // bundled-assets merge supersedes first-party entries afterwards.
        let fs = InMemoryBackend::default();
        for (path, bytes) in importable_tool_bundle_files("aaa-stale") {
            let bytes = if path == "manifest.toml" {
                String::from_utf8(bytes)
                    .unwrap()
                    .replace("\"third_party\"", "\"first_party_requested\"")
                    .into_bytes()
            } else {
                bytes
            };
            fs.write_file(
                &VirtualPath::new(format!("/system/extensions/aaa-stale/{path}")).unwrap(),
                &bytes,
            )
            .await
            .unwrap();
        }
        for (path, bytes) in importable_tool_bundle_files("uploaded-tool") {
            fs.write_file(
                &VirtualPath::new(format!("/system/extensions/uploaded-tool/{path}")).unwrap(),
                &bytes,
            )
            .await
            .unwrap();
        }

        let catalog = AvailableExtensionCatalog::from_filesystem_root(
            &fs,
            &VirtualPath::new("/system/extensions").unwrap(),
            &[],
        )
        .await
        .expect("one stale manifest must not abort the catalog load");

        assert!(
            catalog
                .search("")
                .any(|package| package.package_ref.id.as_str() == "uploaded-tool"),
            "valid extension sorted after the stale one must still load"
        );
        assert!(
            !catalog
                .search("")
                .any(|package| package.package_ref.id.as_str() == "aaa-stale"),
            "stale first-party-trust manifest must be skipped"
        );
    }

    #[tokio::test]
    async fn filesystem_catalog_still_fails_closed_on_transient_manifest_read_error() {
        // Per-entry fail-open is only for validation failures; infrastructure
        // IO errors must keep aborting the load so a flaky volume does not
        // silently drop installed extensions.
        let error = AvailableExtensionCatalog::from_filesystem_root(
            &UnreadableManifestFilesystem,
            &VirtualPath::new("/system/extensions").unwrap(),
            &[],
        )
        .await
        .expect_err("transient manifest read error must abort the catalog load");
        assert!(matches!(error, ProductWorkflowError::Transient { .. }));
    }

    struct UnreadableManifestFilesystem;

    #[async_trait]
    impl RootFilesystem for UnreadableManifestFilesystem {
        async fn list_dir(&self, _path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
            Ok(vec![DirEntry {
                name: "broken-ext".to_string(),
                path: VirtualPath::new("/system/extensions/broken-ext").unwrap(),
                file_type: FileType::Directory,
            }])
        }

        async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
            Err(FilesystemError::NotFound {
                path: path.clone(),
                operation: FilesystemOperation::Stat,
            })
        }

        async fn read_file(&self, path: &VirtualPath) -> Result<Vec<u8>, FilesystemError> {
            Err(FilesystemError::Backend {
                path: path.clone(),
                operation: FilesystemOperation::ReadFile,
                reason: "disk unavailable".to_string(),
            })
        }
    }

    fn importable_tool_bundle_files(id: &str) -> Vec<(String, Vec<u8>)> {
        let manifest = format!(
            r#"
schema_version = "reborn.extension_manifest.v2"
id = "{id}"
name = "Imported Tool"
version = "0.1.0"
description = "Uploaded tool bundle fixture"
trust = "third_party"

[runtime]
kind = "wasm"
module = "wasm/tool.wasm"

[[host_api]]
id = "ironclaw.capability_provider/v1"
section = "capability_provider.tools"

[capability_provider.tools]

[[capability_provider.tools.capabilities]]
id = "{id}.run"
description = "Run the tool"
effects = ["dispatch_capability"]
default_permission = "allow"
visibility = "model"
input_schema_ref = "schemas/run.input.json"
output_schema_ref = "schemas/run.output.json"
prompt_doc_ref = "prompts/run.md"
"#
        );
        vec![
            ("manifest.toml".to_string(), manifest.into_bytes()),
            ("wasm/tool.wasm".to_string(), b"\0asm\x0d\0\x01\0".to_vec()),
            ("schemas/run.input.json".to_string(), b"{}".to_vec()),
            ("schemas/run.output.json".to_string(), b"{}".to_vec()),
            ("prompts/run.md".to_string(), b"# run".to_vec()),
        ]
    }

    #[test]
    fn imported_extension_package_rejects_core_module_wasm() {
        let files = importable_tool_bundle_files("uploaded-tool")
            .into_iter()
            .map(|(path, bytes)| {
                if path == "wasm/tool.wasm" {
                    (path, b"\0asm\x01\0\0\0".to_vec())
                } else {
                    (path, bytes)
                }
            })
            .collect::<Vec<_>>();
        let error = imported_extension_package(files, &[])
            .expect_err("core-module wasm must be rejected at import");
        assert!(format!("{error}").contains("not a WASI component"));
    }

    #[test]
    fn test_tool_fixture_manifests_stay_importable() {
        for (label, manifest) in [
            (
                "market-data",
                include_str!("../../../../test-tools/market-data/manifest.toml"),
            ),
            (
                "hacker-news",
                include_str!("../../../../test-tools/hacker-news/manifest.toml"),
            ),
            (
                "ascii-renderer",
                include_str!("../../../../test-tools/ascii-renderer/manifest.toml"),
            ),
        ] {
            let host_ports =
                ironclaw_host_runtime::default_host_port_catalog().expect("host port catalog");
            let contracts =
                product_extension_host_api_contract_registry().expect("host API contracts");
            let record = ExtensionManifestRecord::from_toml(
                manifest,
                ManifestSource::InstalledLocal,
                &host_ports,
                None,
                &contracts,
            )
            .unwrap_or_else(|error| panic!("test-tools/{label} manifest must validate: {error}"));
            assert_eq!(record.manifest().runtime.kind(), RuntimeKind::Wasm);
        }
    }

    #[test]
    fn imported_extension_package_validates_as_installed_local() {
        let package =
            imported_extension_package(importable_tool_bundle_files("uploaded-tool"), &[])
                .expect("complete wasm tool bundle must import");
        assert_eq!(package.source, ManifestSource::InstalledLocal);
        assert_eq!(package.package_ref.id.as_str(), "uploaded-tool");
    }

    #[tokio::test]
    async fn imported_bundle_reloads_as_installed_local_after_restart() {
        let fs = InMemoryBackend::default();
        let package =
            imported_extension_package(importable_tool_bundle_files("uploaded-tool"), &[])
                .expect("complete wasm tool bundle must import");
        materialize_available_extension(&fs, &package)
            .await
            .expect("materialize uploaded bundle");
        let catalog = AvailableExtensionCatalog::from_filesystem_root(
            &fs,
            &VirtualPath::new("/system/extensions").unwrap(),
            &[],
        )
        .await
        .expect("catalog reload from filesystem");
        let reloaded = catalog
            .search("")
            .find(|package| package.package_ref.id.as_str() == "uploaded-tool")
            .expect("uploaded tool must survive restart reload");
        assert_eq!(reloaded.source, ManifestSource::InstalledLocal);
    }

    #[test]
    fn imported_extension_package_rejects_first_party_trust_claims() {
        let error = imported_extension_package(
            vec![(
                "manifest.toml".to_string(),
                include_str!(
                    "../../../ironclaw_first_party_extensions/assets/github/manifest.toml"
                )
                .as_bytes()
                .to_vec(),
            )],
            &[],
        )
        .expect_err("first-party trust claims must be rejected");
        // The bundled github manifest is v3, so the rejection comes from the
        // v3 reader's source/trust gate ("trust `FirstPartyRequested` is not
        // allowed for this manifest source"); the v2 wording is "not allowed
        // to assert trust". Pin the shared semantic: the error names trust
        // and ties the rejection to the manifest source.
        let message = format!("{error}");
        assert!(
            message.contains("trust") && message.contains("not allowed for this manifest source"),
            "import must reject the first-party trust claim for an InstalledLocal source: {error}"
        );
    }

    #[test]
    fn imported_extension_package_rejects_reserved_host_bundled_extension_ids() {
        let error = imported_extension_package(
            importable_tool_bundle_files("github"),
            &["github".to_string()],
        )
        .expect_err("reserved ids must be rejected");
        assert!(format!("{error}").contains("reserved"));
    }

    #[test]
    fn imported_extension_package_rejects_non_wasm_runtimes() {
        let manifest = r#"
schema_version = "reborn.extension_manifest.v2"
id = "uploaded-mcp"
name = "Uploaded MCP"
version = "0.1.0"
description = "MCP runtime uploaded as a tool bundle"
trust = "third_party"

[runtime]
kind = "mcp"
transport = "http"
url = "https://mcp.example/api"

[[host_api]]
id = "ironclaw.capability_provider/v1"
section = "capability_provider.tools"

[capability_provider.tools]

[[capability_provider.tools.capabilities]]
id = "uploaded-mcp.run"
description = "Run the tool"
effects = ["dispatch_capability"]
default_permission = "allow"
visibility = "model"
input_schema_ref = "schemas/run.input.json"
output_schema_ref = "schemas/run.output.json"
"#;
        let error = imported_extension_package(
            vec![("manifest.toml".to_string(), manifest.as_bytes().to_vec())],
            &[],
        )
        .expect_err("non-wasm runtimes must be rejected");
        assert!(format!("{error}").contains("wasm runtime"));
    }

    #[test]
    fn imported_extension_package_rejects_missing_declared_assets() {
        for missing in [
            "wasm/tool.wasm",
            "schemas/run.input.json",
            "schemas/run.output.json",
            "prompts/run.md",
        ] {
            let files = importable_tool_bundle_files("uploaded-tool")
                .into_iter()
                .filter(|(path, _)| path != missing)
                .collect::<Vec<_>>();
            let error = imported_extension_package(files, &[])
                .expect_err("missing declared assets must be rejected at import");
            let message = format!("{error}");
            assert!(
                message.contains("missing manifest-declared asset") && message.contains(missing),
                "unexpected error for missing `{missing}`: {error}"
            );
        }
    }

    struct MismatchedAssetPathFilesystem;

    #[async_trait]
    impl RootFilesystem for MismatchedAssetPathFilesystem {
        async fn list_dir(&self, _path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
            Ok(vec![DirEntry {
                name: "asset.txt".to_string(),
                path: VirtualPath::new("/system/extensions/other/asset.txt").unwrap(),
                file_type: FileType::File,
            }])
        }

        async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
            Err(FilesystemError::NotFound {
                path: path.clone(),
                operation: FilesystemOperation::Stat,
            })
        }

        async fn read_file(&self, _path: &VirtualPath) -> Result<Vec<u8>, FilesystemError> {
            Ok(b"asset".to_vec())
        }
    }
}
