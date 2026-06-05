use ironclaw_extensions::{ExtensionManifest, ExtensionPackage, ManifestSource};
use ironclaw_host_api::VirtualPath;
use ironclaw_product_workflow::LifecyclePackageKind;

use crate::available_extensions::{
    AvailableExtensionAsset, AvailableExtensionAssetContent, AvailableExtensionPackage,
};

use super::catalog::{package_ref, validate_hub_name};
use super::errors::install_error;
use super::model::{
    GENERIC_TOOL_INPUT_SCHEMA, GENERIC_TOOL_OUTPUT_SCHEMA, IronHubCommandError, IronHubToolEntry,
};

pub(super) fn ironhub_tool_package(
    entry: &IronHubToolEntry,
    wasm: &[u8],
    capabilities: &[u8],
) -> Result<AvailableExtensionPackage, IronHubCommandError> {
    validate_hub_name(&entry.name)?;
    let manifest_toml = generic_tool_manifest(entry);
    let root = VirtualPath::new(format!("/system/extensions/{}", entry.name))
        .map_err(|error| install_error(error.to_string()))?;
    let host_ports = ironclaw_host_runtime::default_host_port_catalog()
        .map_err(|error| install_error(error.to_string()))?;
    let contracts = ironclaw_host_runtime::default_host_api_contract_registry()
        .map_err(|error| install_error(error.to_string()))?;
    let manifest = ExtensionManifest::parse_with_optional_host_api_contracts(
        &manifest_toml,
        ManifestSource::RegistryInstalled,
        &host_ports,
        &contracts,
    )
    .map_err(|error| install_error(error.to_string()))?;
    let package = ExtensionPackage::from_manifest_toml(manifest, root, &manifest_toml)
        .map_err(|error| install_error(error.to_string()))?;
    let package_ref = package_ref(LifecyclePackageKind::Extension, &entry.name)?;
    Ok(AvailableExtensionPackage {
        package_ref,
        manifest_toml,
        package,
        assets: vec![
            bytes_asset("manifest.toml", manifest_toml_bytes(entry).as_slice()),
            bytes_asset(&format!("wasm/{}_tool.wasm", entry.name), wasm),
            bytes_asset("legacy/capabilities.json", capabilities),
            bytes_asset(
                &format!("schemas/{}/invoke.input.v1.json", entry.name),
                GENERIC_TOOL_INPUT_SCHEMA,
            ),
            bytes_asset(
                &format!("schemas/{}/raw_output.v1.json", entry.name),
                GENERIC_TOOL_OUTPUT_SCHEMA,
            ),
        ],
    })
}

fn manifest_toml_bytes(entry: &IronHubToolEntry) -> Vec<u8> {
    generic_tool_manifest(entry).into_bytes()
}

fn generic_tool_manifest(entry: &IronHubToolEntry) -> String {
    format!(
        r#"schema_version = "reborn.extension_manifest.v2"
id = "{id}"
name = "{name}"
version = "{version}"
description = "{description}"
trust = "third_party"

[runtime]
kind = "wasm"
module = "wasm/{id}_tool.wasm"

[[host_api]]
id = "ironclaw.capability_provider/v1"
section = "capability_provider.tools"

[capability_provider.tools]

[[capability_provider.tools.capabilities]]
id = "{id}.invoke"
description = "{description}"
effects = ["dispatch_capability", "network"]
default_permission = "ask"
visibility = "model"
input_schema_ref = "schemas/{id}/invoke.input.v1.json"
output_schema_ref = "schemas/{id}/raw_output.v1.json"
required_host_ports = ["host.runtime.http_egress"]
"#,
        id = toml_escape(&entry.name),
        name = toml_escape(&entry.name),
        version = toml_escape(&entry.version),
        description = toml_escape(&entry.description),
    )
}

fn bytes_asset(path: &str, bytes: &[u8]) -> AvailableExtensionAsset {
    AvailableExtensionAsset {
        path: path.to_string(),
        content: AvailableExtensionAssetContent::Bytes(bytes.to_vec()),
    }
}

fn toml_escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}
