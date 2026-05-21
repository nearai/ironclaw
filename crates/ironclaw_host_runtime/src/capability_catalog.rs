use std::collections::HashMap;

use ironclaw_extensions::{CapabilityVisibility, ExtensionPackage, ExtensionRegistry};
use ironclaw_filesystem::{FilesystemError, RootFilesystem};
use ironclaw_host_api::{
    CapabilityDescriptor, CapabilityId, CapabilityProfileSchemaRef, VirtualPath,
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
    /// Resolved prompt document. Required for model-visible capabilities.
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

        let input_schema = read_json_ref(
            fs,
            &package.root,
            &declaration.input_schema_ref,
            "input_schema_ref",
        )
        .await?;
        let output_schema = read_json_ref(
            fs,
            &package.root,
            &declaration.output_schema_ref,
            "output_schema_ref",
        )
        .await?;
        let prompt_doc = match &declaration.prompt_doc_ref {
            Some(prompt_ref) => Some(read_text_ref(fs, &package.root, prompt_ref).await?),
            None if declaration.visibility == CapabilityVisibility::Model => {
                return Err(HostRuntimeError::invalid_request(format!(
                    "model-visible capability {} is missing prompt_doc_ref",
                    declaration.id
                )));
            }
            None => None,
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

async fn read_json_ref<F>(
    fs: &F,
    root: &VirtualPath,
    reference: &CapabilityProfileSchemaRef,
    field: &'static str,
) -> Result<Value, HostRuntimeError>
where
    F: RootFilesystem,
{
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
    F: RootFilesystem,
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
    F: RootFilesystem,
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
    let message = match error {
        FilesystemError::NotFound { .. } => {
            format!("missing {field} at {}", path.as_str())
        }
        _ => format!("failed to read {field} at {}", path.as_str()),
    };
    HostRuntimeError::invalid_request(message)
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
