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
    let manifest_asset = bytes_asset("manifest.toml", manifest_toml.as_bytes());
    Ok(AvailableExtensionPackage {
        package_ref,
        manifest_toml,
        package,
        assets: vec![
            manifest_asset,
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

fn generic_tool_manifest(entry: &IronHubToolEntry) -> String {
    format!(
        r#"schema_version = "reborn.extension_manifest.v2"
id = {id}
name = {name}
version = {version}
description = {description}
trust = "third_party"

[runtime]
kind = "wasm"
module = {module}

[[host_api]]
id = "ironclaw.capability_provider/v1"
section = "capability_provider.tools"

[capability_provider.tools]

[[capability_provider.tools.capabilities]]
id = {capability_id}
description = {description}
effects = ["dispatch_capability", "network"]
default_permission = "ask"
visibility = "model"
input_schema_ref = {input_schema_ref}
output_schema_ref = {output_schema_ref}
required_host_ports = ["host.runtime.http_egress"]
"#,
        id = toml_string(&entry.name),
        name = toml_string(&entry.name),
        version = toml_string(&entry.version),
        description = toml_string(&entry.description),
        module = toml_string(format!("wasm/{}_tool.wasm", entry.name)),
        capability_id = toml_string(format!("{}.invoke", entry.name)),
        input_schema_ref = toml_string(format!("schemas/{}/invoke.input.v1.json", entry.name)),
        output_schema_ref = toml_string(format!("schemas/{}/raw_output.v1.json", entry.name)),
    )
}

fn bytes_asset(path: &str, bytes: &[u8]) -> AvailableExtensionAsset {
    AvailableExtensionAsset {
        path: path.to_string(),
        content: AvailableExtensionAssetContent::Bytes(bytes.to_vec()),
    }
}

fn toml_string(value: impl Into<String>) -> String {
    toml::Value::String(value.into()).to_string()
}

#[cfg(test)]
mod tests {
    use super::generic_tool_manifest;
    use crate::ironhub::model::{IronHubArtifact, IronHubProvenance, IronHubToolEntry};

    #[test]
    fn generic_tool_manifest_uses_toml_escaping_for_catalog_strings() {
        let manifest = generic_tool_manifest(&IronHubToolEntry {
            name: "quote_tool".to_string(),
            crate_name: "quote_tool".to_string(),
            version: "0.1.0".to_string(),
            description: "quote \" slash \\ newline\nok".to_string(),
            provenance: IronHubProvenance::Official,
            wasm: IronHubArtifact {
                url: "https://hub.ironclaw.com/quote_tool.wasm".to_string(),
                size_bytes: 1,
                sha256: "a".repeat(64),
            },
            capabilities: IronHubArtifact {
                url: "https://hub.ironclaw.com/quote_tool.capabilities.json".to_string(),
                size_bytes: 1,
                sha256: "b".repeat(64),
            },
        });

        let parsed: toml::Value = toml::from_str(&manifest).expect("manifest TOML parses");
        assert_eq!(parsed["id"].as_str(), Some("quote_tool"));
        assert_eq!(
            parsed["description"].as_str(),
            Some("quote \" slash \\ newline\nok")
        );
        assert_eq!(
            parsed["runtime"]["module"].as_str(),
            Some("wasm/quote_tool_tool.wasm")
        );
        assert_eq!(
            parsed["capability_provider"]["tools"]["capabilities"][0]["id"].as_str(),
            Some("quote_tool.invoke")
        );
    }
}
