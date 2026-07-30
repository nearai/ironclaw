use crate::{AvailableExtensionPackage, registry_extension_package};

use super::catalog::validate_hub_name;
use super::model::{
    GENERIC_TOOL_INPUT_SCHEMA, GENERIC_TOOL_OUTPUT_SCHEMA, IronHubCommandError, IronHubToolEntry,
};

pub(crate) fn ironhub_tool_package(
    entry: &IronHubToolEntry,
    wasm: Vec<u8>,
    capabilities: Vec<u8>,
    reserved_bundled_ids: &[String],
) -> Result<AvailableExtensionPackage, IronHubCommandError> {
    validate_hub_name(&entry.name)?;
    validate_hub_name(&entry.crate_name)?;
    let module_path = format!("wasm/{}_tool.wasm", entry.crate_name);
    let input_schema_path = format!("schemas/{}/invoke.input.v1.json", entry.name);
    let output_schema_path = format!("schemas/{}/raw_output.v1.json", entry.name);
    let manifest =
        generic_tool_manifest(entry, &module_path, &input_schema_path, &output_schema_path);
    registry_extension_package(
        vec![
            ("manifest.toml".to_string(), manifest.into_bytes()),
            (module_path, wasm),
            ("legacy/capabilities.json".to_string(), capabilities),
            (input_schema_path, GENERIC_TOOL_INPUT_SCHEMA.to_vec()),
            (output_schema_path, GENERIC_TOOL_OUTPUT_SCHEMA.to_vec()),
        ],
        reserved_bundled_ids,
    )
    .map_err(IronHubCommandError::Product)
}

fn generic_tool_manifest(
    entry: &IronHubToolEntry,
    module_path: &str,
    input_schema_path: &str,
    output_schema_path: &str,
) -> String {
    format!(
        r#"schema_version = "reborn.extension_manifest.v3"
id = {id}
name = {name}
version = {version}
description = {description}
trust = "third_party"

[runtime]
kind = "wasm"
module = {module}

[[tools]]
origin_gate_matrix = {{ loop_run = "gated_unless_granted", product = "forbidden", automation = "forbidden" }}
id = {capability_id}
description = {description}
effects = ["network"]
default_permission = "ask"
visibility = "model"
input_schema_ref = {input_schema_ref}
output_schema_ref = {output_schema_ref}
"#,
        id = toml_string(&entry.name),
        name = toml_string(&entry.name),
        version = toml_string(&entry.version),
        description = toml_string(&entry.description),
        module = toml_string(module_path),
        capability_id = toml_string(format!("{}.invoke", entry.name)),
        input_schema_ref = toml_string(input_schema_path),
        output_schema_ref = toml_string(output_schema_path),
    )
}

fn toml_string(value: impl Into<String>) -> String {
    toml::Value::String(value.into()).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ironhub::model::{IronHubArtifact, IronHubProvenance};

    #[test]
    fn generic_tool_manifest_uses_current_v3_extension_contract() {
        let entry = IronHubToolEntry {
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
        };

        let manifest = generic_tool_manifest(
            &entry,
            "wasm/quote_tool_tool.wasm",
            "schemas/quote_tool/invoke.input.v1.json",
            "schemas/quote_tool/raw_output.v1.json",
        );
        let parsed: toml::Value = toml::from_str(&manifest).expect("manifest TOML parses");
        assert_eq!(
            parsed["schema_version"].as_str(),
            Some("reborn.extension_manifest.v3")
        );
        assert_eq!(
            parsed["description"].as_str(),
            Some("quote \" slash \\ newline\nok")
        );
        assert_eq!(parsed["tools"][0]["id"].as_str(), Some("quote_tool.invoke"));
    }
}
