use std::collections::HashMap;

use ironclaw_extension_registry::{
    CapabilityDescriptorSchemaMode, CapabilityVisibility, ExtensionPackage, ExtensionRegistry,
};
use ironclaw_filesystem::{FilesystemError, RootFilesystem};
use ironclaw_host_api::{
    capability::CapabilityDescriptor,
    capability_profile::CapabilityProfileSchemaRef,
    ids::CapabilityId,
    messaging::{STANDARD_SCHEMA_REF_PREFIX, resolve_standard_schema_ref},
    path::VirtualPath,
};
use serde_json::Value;

use crate::HostRuntimeError;

pub const MAX_HOT_SCHEMA_BYTES: usize = 64 * 1024;
pub const MAX_HOT_PROMPT_BYTES: usize = 16 * 1024;

/// Resolved, model-facing capability catalog derived from cold extension manifests.
///
/// This catalog is publication metadata only. It does not grant authority and it
/// does not execute extension code.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HotCapabilityCatalog {
    pub capabilities: Vec<HotCapabilityRecord>,
}

impl HotCapabilityCatalog {
    pub fn get(&self, id: &CapabilityId) -> Option<&HotCapabilityRecord> {
        self.capabilities
            .iter()
            .find(|record| &record.descriptor.id == id)
    }
}

/// One resolved capability record safe for hot surface publication.
#[derive(Debug, Clone, PartialEq)]
pub struct HotCapabilityRecord {
    /// Descriptor with `parameters_schema` replaced by resolved input schema.
    pub descriptor: CapabilityDescriptor,
    /// Resolved output schema retained adjacent to the descriptor.
    pub output_schema: Value,
    /// Optional lazy help document. Not part of the always-visible model surface.
    pub prompt_doc: Option<String>,
}

pub async fn publish_hot_capability_catalog<F>(
    fs: &F,
    registry: &ExtensionRegistry,
) -> Result<HotCapabilityCatalog, HostRuntimeError>
where
    F: RootFilesystem,
{
    let mut records = Vec::new();
    for package in registry.extensions() {
        publish_package_capabilities(fs, package, &mut records).await?;
    }
    Ok(HotCapabilityCatalog {
        capabilities: records,
    })
}

async fn publish_package_capabilities<F>(
    fs: &F,
    package: &ExtensionPackage,
    records: &mut Vec<HotCapabilityRecord>,
) -> Result<(), HostRuntimeError>
where
    F: RootFilesystem,
{
    let declarations_by_id: HashMap<_, _> = package
        .manifest
        .capabilities
        .iter()
        .map(|declaration| (&declaration.id, declaration))
        .collect();

    for descriptor in &package.capabilities {
        let declaration = declarations_by_id
            .get(&descriptor.id)
            .copied()
            .ok_or_else(|| {
                HostRuntimeError::invalid_request(format!(
                    "capability {} is missing manifest declaration",
                    descriptor.id
                ))
            })?;
        if declaration.visibility != CapabilityVisibility::Model {
            continue;
        }

        let (input_schema, output_schema, prompt_doc) = if package.descriptor_schema_mode
            == CapabilityDescriptorSchemaMode::InlineDynamic
        {
            // Hosted MCP inline schemas are remote-server-provided, i.e.
            // attacker-influenced. Mirror the filesystem branch's
            // `jsonschema::validator_for` check before this schema reaches
            // model-facing publication.
            jsonschema::validator_for(&descriptor.parameters_schema).map_err(|error| {
                HostRuntimeError::invalid_request(format!(
                    "capability {} inline parameters_schema must contain valid JSON schema: {error}",
                    descriptor.id
                ))
            })?;
            (
                descriptor.parameters_schema.clone(),
                Value::Object(serde_json::Map::new()),
                None,
            )
        } else {
            let root = package.materialized_root().map_err(|error| {
                HostRuntimeError::invalid_request(format!(
                    "capability {} requires package filesystem schemas: {error}",
                    descriptor.id
                ))
            })?;
            let input_schema =
                read_json_ref(fs, root, &declaration.input_schema_ref, "input_schema_ref").await?;
            let output_schema = match &declaration.output_schema_ref {
                Some(reference) => read_json_ref(fs, root, reference, "output_schema_ref").await?,
                None => Value::Object(serde_json::Map::new()),
            };
            let prompt_doc = match &declaration.prompt_doc_ref {
                Some(prompt_ref) => Some(read_text_ref(fs, root, prompt_ref).await?),
                None => None,
            };
            (input_schema, output_schema, prompt_doc)
        };

        let mut hot_descriptor = descriptor.clone();
        hot_descriptor.parameters_schema = input_schema;
        records.push(HotCapabilityRecord {
            descriptor: hot_descriptor,
            output_schema,
            prompt_doc,
        });
    }
    Ok(())
}

pub(crate) async fn read_json_ref<F>(
    fs: &F,
    root: &VirtualPath,
    reference: &CapabilityProfileSchemaRef,
    field: &'static str,
) -> Result<Value, HostRuntimeError>
where
    F: RootFilesystem + ?Sized,
{
    // A standard-bound tool's schema lives in the compiled-in messaging
    // registry (`ironclaw_host_api::messaging`), never on the package's
    // filesystem root. This is the single choke point both `input_schema_ref`
    // and `output_schema_ref` reads go through, so gating here (rather than
    // at each call site) resolves both without duplicating the prefix check.
    // Must run before `resolve_under_root`/the filesystem read below: a
    // `standard:` ref can never exist on disk, so falling through would hit
    // the filesystem for a path that can never resolve there instead of
    // failing closed with the ref named.
    if reference.as_str().starts_with(STANDARD_SCHEMA_REF_PREFIX) {
        return match resolve_standard_schema_ref(reference.as_str()) {
            Some(raw) => {
                let schema = serde_json::from_str(raw).map_err(|error| {
                    HostRuntimeError::invalid_request(format!(
                        "{field} {} must contain valid JSON schema: {error}",
                        reference.as_str()
                    ))
                })?;
                jsonschema::options()
                    .should_validate_formats(true)
                    .build(&schema)
                    .map_err(|error| {
                        HostRuntimeError::invalid_request(format!(
                            "{field} {} must contain valid JSON schema: {error}",
                            reference.as_str()
                        ))
                    })?;
                Ok(schema)
            }
            None => Err(HostRuntimeError::invalid_request(format!(
                "{field} {} references unknown standard schema",
                reference.as_str()
            ))),
        };
    }
    let path = resolve_under_root(root, reference)?;
    let bytes = read_bounded(fs, &path, MAX_HOT_SCHEMA_BYTES, field).await?;
    let schema = serde_json::from_slice(&bytes).map_err(|error| {
        HostRuntimeError::invalid_request(format!(
            "{field} {} must contain valid JSON schema: {error}",
            reference.as_str()
        ))
    })?;
    jsonschema::validator_for(&schema).map_err(|error| {
        HostRuntimeError::invalid_request(format!(
            "{field} {} must contain valid JSON schema: {error}",
            reference.as_str()
        ))
    })?;
    Ok(schema)
}

async fn read_text_ref<F>(
    fs: &F,
    root: &VirtualPath,
    reference: &CapabilityProfileSchemaRef,
) -> Result<String, HostRuntimeError>
where
    F: RootFilesystem + ?Sized,
{
    let path = resolve_under_root(root, reference)?;
    let bytes = read_bounded(fs, &path, MAX_HOT_PROMPT_BYTES, "prompt_doc_ref").await?;
    String::from_utf8(bytes).map_err(|error| {
        HostRuntimeError::invalid_request(format!(
            "prompt_doc_ref {} must be valid UTF-8: {error}",
            reference.as_str()
        ))
    })
}

async fn read_bounded<F>(
    fs: &F,
    path: &VirtualPath,
    max_bytes: usize,
    field: &'static str,
) -> Result<Vec<u8>, HostRuntimeError>
where
    F: RootFilesystem + ?Sized,
{
    let bytes = fs
        .read_file_bounded(path, max_bytes)
        .await
        .map_err(|error| map_read_error(path, field, error))?
        .ok_or_else(|| {
            HostRuntimeError::invalid_request(format!(
                "{field} at {} exceeds {max_bytes} bytes",
                path.as_str()
            ))
        })?;
    if bytes.len() > max_bytes {
        return Err(HostRuntimeError::invalid_request(format!(
            "{field} at {} exceeds {max_bytes} bytes",
            path.as_str()
        )));
    }
    Ok(bytes)
}

fn map_read_error(
    path: &VirtualPath,
    field: &'static str,
    error: FilesystemError,
) -> HostRuntimeError {
    // Isolate (`InvalidRequest`): the declared reference itself is bad — a
    // missing file means the manifest points at something that does not
    // exist, which is a per-capability defect, not a system-wide one.
    //
    // Abort (`Unavailable`): the backend itself is broken. Folding these into
    // `InvalidRequest` would make a genuine storage outage indistinguishable
    // from "one bad schema ref" and let per-capability isolation upstream
    // silently swallow every capability in the surface instead of failing
    // the call loudly.
    match error {
        FilesystemError::NotFound { .. } => {
            HostRuntimeError::invalid_request(format!("missing {field} at {}", path.as_str()))
        }
        FilesystemError::Backend { .. }
        | FilesystemError::BackendBusy { .. }
        | FilesystemError::MountNotFound { .. }
        | FilesystemError::Contract(_) => HostRuntimeError::unavailable(format!(
            "storage backend unavailable while reading {field} at {}",
            path.as_str()
        )),
        _ => HostRuntimeError::invalid_request(format!(
            "failed to read {field} at {}",
            path.as_str()
        )),
    }
}

fn resolve_under_root(
    root: &VirtualPath,
    reference: &CapabilityProfileSchemaRef,
) -> Result<VirtualPath, HostRuntimeError> {
    validate_relative_manifest_asset_ref(reference)?;
    VirtualPath::new(format!(
        "{}/{}",
        root.as_str().trim_end_matches('/'),
        reference.as_str()
    ))
    .map_err(|error| {
        HostRuntimeError::invalid_request(format!(
            "invalid manifest asset ref {} under {}: {error}",
            reference.as_str(),
            root.as_str()
        ))
    })
}

fn validate_relative_manifest_asset_ref(
    reference: &CapabilityProfileSchemaRef,
) -> Result<(), HostRuntimeError> {
    let value = reference.as_str();
    if value.starts_with('/')
        || value.contains('\\')
        || value.chars().any(|ch| ch == '\0' || ch.is_control())
        || value
            .split('/')
            .any(|segment| segment.is_empty() || segment == "." || segment == "..")
    {
        return Err(HostRuntimeError::invalid_request(format!(
            "invalid manifest asset ref {value}: path traversal characters are not allowed"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_extension_contracts::hosted_mcp::{
        HostedMcpDiscoveredTool, HostedMcpDiscoveredToolAnnotations,
    };
    use ironclaw_extension_registry::{
        CapabilityProviderHostApiContract, HostApiContractRegistry,
        package_with_discovered_hosted_mcp_tools,
    };
    use ironclaw_filesystem::InMemoryBackend;
    use ironclaw_host_api::host_port::HostPortCatalog;

    const HOSTED_MCP_MANIFEST: &str = r#"
schema_version = "reborn.extension_manifest.v2"
id = "hosted-tool"
name = "Hosted Tool"
version = "1.0.0"
description = "Hosted MCP provider"
trust = "third_party"

[runtime]
kind = "mcp"
transport = "http"
url = "https://mcp.example.test/mcp"

[[host_api]]
id = "ironclaw.capability_provider/v1"
section = "capability_provider.tools"

[capability_provider.tools]

[[capability_provider.tools.capabilities]]
id = "hosted-tool.invoke"
description = "Invoke the hosted tool"
effects = ["dispatch_capability", "network"]
default_permission = "ask"
visibility = "model"
input_schema_ref = "schemas/hosted-tool/invoke.input.json"
"#;

    fn contracts() -> HostApiContractRegistry {
        let mut contracts = HostApiContractRegistry::new();
        contracts
            .register(std::sync::Arc::new(
                CapabilityProviderHostApiContract::new().expect("contract"),
            ))
            .expect("register contract");
        contracts
    }

    fn hosted_mcp_package() -> ExtensionPackage {
        let manifest = ironclaw_extension_registry::ExtensionManifest::parse(
            HOSTED_MCP_MANIFEST,
            ironclaw_extension_registry::ManifestSource::HostBundled,
            &HostPortCatalog::default(),
            &contracts(),
        )
        .expect("valid hosted MCP manifest");
        ExtensionPackage::from_manifest(
            manifest,
            VirtualPath::new("/system/extensions/hosted-tool").expect("valid root"),
        )
        .expect("valid hosted MCP package")
    }

    /// Regression test for the review finding that the `InlineDynamic` branch
    /// of `publish_package_capabilities` published a remote-server-provided
    /// inline schema without validation, unlike the sibling filesystem branch
    /// which runs `jsonschema::validator_for`. A hosted MCP server's
    /// `tools/list` response is attacker-influenced input, so an invalid
    /// inline schema must be rejected before model-facing publication.
    #[tokio::test]
    async fn inline_dynamic_schema_is_validated_before_publication() {
        let package = hosted_mcp_package();
        let tools = vec![HostedMcpDiscoveredTool {
            name: "invoke".to_string(),
            description: "Invoke the hosted tool".to_string(),
            input_schema: serde_json::json!({"type": "not-a-json-schema-type"}),
            annotations: HostedMcpDiscoveredToolAnnotations::default(),
        }];
        let discovered = package_with_discovered_hosted_mcp_tools(&package, &tools)
            .expect("build discovered package with inline schema");
        assert_eq!(
            discovered.descriptor_schema_mode,
            CapabilityDescriptorSchemaMode::InlineDynamic
        );

        let mut registry = ExtensionRegistry::new();
        registry.insert(discovered).expect("insert package");
        let fs = InMemoryBackend::new();

        let err = publish_hot_capability_catalog(&fs, &registry)
            .await
            .expect_err("invalid inline schema must be rejected before publication");

        assert!(
            matches!(&err, HostRuntimeError::InvalidRequest { reason }
                if reason.contains("hosted-tool.invoke") && reason.contains("valid JSON schema")),
            "unexpected error: {err:?}"
        );
    }

    /// Regression test: `map_read_error` must reclassify infrastructure
    /// `FilesystemError` variants (`Backend`, `BackendBusy`, `MountNotFound`,
    /// `Contract`) as `HostRuntimeError::Unavailable`, not fold them into
    /// `InvalidRequest` alongside genuinely bad references (`NotFound`).
    /// Before this fix every non-`NotFound` `FilesystemError` mapped to
    /// `InvalidRequest`, so a real storage outage was indistinguishable from
    /// "this schema ref is bad" and per-capability isolation upstream would
    /// have silently swallowed every extension instead of aborting the call.
    #[test]
    fn map_read_error_reclassifies_infrastructure_errors_as_unavailable() {
        let path = VirtualPath::new("/system/extensions/x/schemas/y.json").unwrap();

        let infra_errors = vec![
            FilesystemError::Backend {
                path: path.clone(),
                operation: ironclaw_filesystem::FilesystemOperation::ReadFile,
                reason: "disk offline".to_string(),
            },
            FilesystemError::BackendBusy {
                path: path.clone(),
                operation: ironclaw_filesystem::FilesystemOperation::ReadFile,
            },
            FilesystemError::MountNotFound { path: path.clone() },
        ];
        for error in infra_errors {
            let mapped = map_read_error(&path, "input_schema_ref", error);
            assert!(
                matches!(mapped, HostRuntimeError::Unavailable { .. }),
                "infrastructure filesystem error must map to Unavailable, got {mapped:?}"
            );
        }

        let not_found = FilesystemError::NotFound {
            path: path.clone(),
            operation: ironclaw_filesystem::FilesystemOperation::ReadFile,
        };
        let mapped = map_read_error(&path, "input_schema_ref", not_found);
        assert!(
            matches!(mapped, HostRuntimeError::InvalidRequest { .. }),
            "a missing schema ref is a per-capability defect and must stay InvalidRequest, got {mapped:?}"
        );
    }
}
