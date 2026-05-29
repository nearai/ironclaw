//! Built-in first-party capability handlers.
//!
//! These are host-owned capabilities, not extension-declared tools. They keep
//! pure tool logic behind the Reborn capability path so callers still pass
//! through `CapabilityHost`, trust policy, grants, resource accounting, and
//! runtime dispatch before any handler runs.

mod echo;
mod http;
mod json;
mod schemas;
mod shell;
mod skill_management;
mod skill_url_install;
mod spawn_subagent;
mod time;

use std::{future::Future, panic::AssertUnwindSafe, sync::Arc, time::Instant};

use async_trait::async_trait;
use futures_util::FutureExt as _;
use ironclaw_extensions::{
    CapabilityManifest, CapabilityVisibility, ExtensionError, ExtensionManifest, ExtensionPackage,
    ExtensionRuntime, MANIFEST_SCHEMA_VERSION, ManifestSource,
};
use ironclaw_first_party_extensions::coding::{
    CodingCapabilityError, CodingCapabilityKind, CodingCapabilityRequest, CodingCapabilityState,
};
use ironclaw_host_api::{
    CapabilityId, CapabilityProfileSchemaRef, EffectKind, ExtensionId, HostApiError,
    PermissionMode, RequestedTrustClass, ResourceCeiling, ResourceEstimate, ResourceProfile,
    ResourceUsage, RuntimeDispatchErrorKind, RuntimeHttpEgressError, RuntimeHttpEgressResponse,
    TrustClass, VirtualPath,
};

use crate::{
    FirstPartyCapabilityError, FirstPartyCapabilityHandler, FirstPartyCapabilityRegistry,
    FirstPartyCapabilityRequest, FirstPartyCapabilityResult,
};

pub(crate) use self::schemas::resolve_builtin_input_schema_ref;

pub use echo::ECHO_CAPABILITY_ID;
pub use http::{HTTP_CAPABILITY_ID, HTTP_SAVE_CAPABILITY_ID};
pub use json::JSON_CAPABILITY_ID;
pub use shell::SHELL_CAPABILITY_ID;
pub use skill_management::{
    SKILL_INSTALL_CAPABILITY_ID, SKILL_LIST_CAPABILITY_ID, SKILL_REMOVE_CAPABILITY_ID,
};
pub use spawn_subagent::SPAWN_SUBAGENT_CAPABILITY_ID;
pub use time::TIME_CAPABILITY_ID;

pub const BUILTIN_FIRST_PARTY_PROVIDER: &str = "builtin";
pub const READ_FILE_CAPABILITY_ID: &str = "builtin.read_file";
pub const WRITE_FILE_CAPABILITY_ID: &str = "builtin.write_file";
pub const LIST_DIR_CAPABILITY_ID: &str = "builtin.list_dir";
pub const GLOB_CAPABILITY_ID: &str = "builtin.glob";
pub const GREP_CAPABILITY_ID: &str = "builtin.grep";
pub const APPLY_PATCH_CAPABILITY_ID: &str = "builtin.apply_patch";

const MAX_FIRST_PARTY_INPUT_BYTES: usize = 1_048_576;
const MAX_WRITE_FILE_INPUT_BYTES: usize = 6 * 1024 * 1024;
const MAX_APPLY_PATCH_INPUT_BYTES: usize = 21 * 1024 * 1024;
const FIRST_PARTY_DEFAULT_OUTPUT_BYTES: u64 = 16 * 1024;
const FIRST_PARTY_MAX_OUTPUT_BYTES: u64 = 1_048_576;
const FIRST_PARTY_DEFAULT_WALL_CLOCK_MS: u64 = 100;
const FIRST_PARTY_MAX_WALL_CLOCK_MS: u64 = 5_000;

#[derive(Debug, Clone, Copy)]
struct CodingCapabilityMetadata {
    id: &'static str,
    kind: CodingCapabilityKind,
    description: &'static str,
    effects: &'static [EffectKind],
    max_input_bytes: usize,
}

const CODING_CAPABILITIES: &[CodingCapabilityMetadata] = &[
    CodingCapabilityMetadata {
        id: READ_FILE_CAPABILITY_ID,
        kind: CodingCapabilityKind::ReadFile,
        description: "Read a file through scoped mounts with v1 read_file output shape",
        effects: &[EffectKind::ReadFilesystem],
        max_input_bytes: MAX_FIRST_PARTY_INPUT_BYTES,
    },
    CodingCapabilityMetadata {
        id: WRITE_FILE_CAPABILITY_ID,
        kind: CodingCapabilityKind::WriteFile,
        description: "Write content through scoped mounts with v1 write_file output shape",
        effects: &[EffectKind::WriteFilesystem],
        max_input_bytes: MAX_WRITE_FILE_INPUT_BYTES,
    },
    CodingCapabilityMetadata {
        id: LIST_DIR_CAPABILITY_ID,
        kind: CodingCapabilityKind::ListDir,
        description: "List directory contents through scoped mounts with v1 list_dir output shape",
        effects: &[EffectKind::ReadFilesystem],
        max_input_bytes: MAX_FIRST_PARTY_INPUT_BYTES,
    },
    CodingCapabilityMetadata {
        id: GLOB_CAPABILITY_ID,
        kind: CodingCapabilityKind::Glob,
        description: "Find files under a scoped directory with v1 glob output shape",
        effects: &[EffectKind::ReadFilesystem],
        max_input_bytes: MAX_FIRST_PARTY_INPUT_BYTES,
    },
    CodingCapabilityMetadata {
        id: GREP_CAPABILITY_ID,
        kind: CodingCapabilityKind::Grep,
        description: "Search scoped file contents with v1 grep output modes",
        effects: &[EffectKind::ReadFilesystem],
        max_input_bytes: MAX_FIRST_PARTY_INPUT_BYTES,
    },
    CodingCapabilityMetadata {
        id: APPLY_PATCH_CAPABILITY_ID,
        kind: CodingCapabilityKind::ApplyPatch,
        description: "Apply exact/fuzzy search-replace edits through scoped mounts",
        effects: &[EffectKind::ReadFilesystem, EffectKind::WriteFilesystem],
        max_input_bytes: MAX_APPLY_PATCH_INPUT_BYTES,
    },
];

/// Create the host-assigned package that declares built-in first-party
/// capabilities for the capability surface.
pub fn builtin_first_party_package() -> Result<ExtensionPackage, ExtensionError> {
    ExtensionPackage::from_manifest(
        ExtensionManifest {
            schema_version: MANIFEST_SCHEMA_VERSION.to_string(),
            id: ExtensionId::new(BUILTIN_FIRST_PARTY_PROVIDER)?,
            name: "Built-in first-party capabilities".to_string(),
            version: "0.1.0".to_string(),
            description: "Host-owned built-in Reborn capabilities".to_string(),
            source: ManifestSource::HostBundled,
            requested_trust: RequestedTrustClass::FirstPartyRequested,
            // Effective first-party trust is assigned by host policy at
            // invocation/surface time. Descriptor trust stays conservative.
            descriptor_trust_default: TrustClass::Sandbox,
            runtime: ExtensionRuntime::FirstParty {
                service: "builtin".to_string(),
            },
            host_apis: Vec::new(),
            capabilities: {
                let mut capabilities = vec![
                    echo::manifest()?,
                    time::manifest()?,
                    json::manifest()?,
                    http::manifest()?,
                    http::save_manifest()?,
                    shell::manifest()?,
                    spawn_subagent::manifest()?,
                ];
                capabilities.extend(coding_manifests()?);
                capabilities.extend(skill_management::manifests()?);
                capabilities
            },
        },
        VirtualPath::new("/system/extensions/builtin")?,
    )
}

fn coding_manifests() -> Result<Vec<CapabilityManifest>, ExtensionError> {
    CODING_CAPABILITIES
        .iter()
        .map(|metadata| {
            first_party_capability_manifest(
                metadata.id,
                metadata.description,
                metadata.effects.to_vec(),
                PermissionMode::Allow,
                resource_profile(),
            )
        })
        .collect()
}

/// Create handlers for all built-in first-party capabilities.
pub fn builtin_first_party_handlers() -> Result<FirstPartyCapabilityRegistry, HostApiError> {
    let handler = Arc::new(BuiltinFirstPartyTools::default());
    let mut registry = FirstPartyCapabilityRegistry::new()
        .with_handler(CapabilityId::new(ECHO_CAPABILITY_ID)?, handler.clone())
        .with_handler(CapabilityId::new(TIME_CAPABILITY_ID)?, handler.clone())
        .with_handler(CapabilityId::new(JSON_CAPABILITY_ID)?, handler.clone())
        .with_handler(CapabilityId::new(HTTP_CAPABILITY_ID)?, handler.clone())
        .with_handler(CapabilityId::new(HTTP_SAVE_CAPABILITY_ID)?, handler.clone())
        .with_handler(CapabilityId::new(SHELL_CAPABILITY_ID)?, handler.clone());
    for metadata in CODING_CAPABILITIES {
        registry.insert_handler(CapabilityId::new(metadata.id)?, handler.clone());
    }
    registry.insert_handler(
        CapabilityId::new(SPAWN_SUBAGENT_CAPABILITY_ID)?,
        handler.clone(),
    );
    skill_management::insert_handlers(&mut registry)?;
    Ok(registry)
}

fn first_party_capability_manifest(
    id: &str,
    description: &str,
    effects: Vec<EffectKind>,
    default_permission: PermissionMode,
    resource_profile: Option<ResourceProfile>,
) -> Result<CapabilityManifest, ExtensionError> {
    let schema_name = id.strip_prefix("builtin.").unwrap_or(id).replace('.', "-");
    Ok(CapabilityManifest {
        id: CapabilityId::new(id)?,
        implements: Vec::new(),
        description: description.to_string(),
        effects,
        default_permission,
        visibility: CapabilityVisibility::Model,
        input_schema_ref: CapabilityProfileSchemaRef::new(format!(
            "schemas/builtin/{schema_name}.input.v1.json"
        ))?,
        output_schema_ref: CapabilityProfileSchemaRef::new(format!(
            "schemas/builtin/{schema_name}.output.v1.json"
        ))?,
        prompt_doc_ref: None,
        required_host_ports: Vec::new(),
        runtime_credentials: Vec::new(),
        resource_profile,
    })
}

#[derive(Debug, Default)]
pub struct BuiltinFirstPartyTools {
    coding_state: CodingCapabilityState,
}

#[async_trait]
impl FirstPartyCapabilityHandler for BuiltinFirstPartyTools {
    async fn dispatch(
        &self,
        mut request: FirstPartyCapabilityRequest,
    ) -> Result<FirstPartyCapabilityResult, FirstPartyCapabilityError> {
        bounded_input_size(request.capability_id.as_str(), &request.input)?;
        normalize_optional_null_sentinels(&mut request);
        let start = Instant::now();
        let mut network_egress_bytes = 0;
        let output = match request.capability_id.as_str() {
            ECHO_CAPABILITY_ID => echo::dispatch(&request.input)?,
            TIME_CAPABILITY_ID => time::dispatch(&request.input)?,
            JSON_CAPABILITY_ID => json::dispatch(&request.input)?,
            HTTP_CAPABILITY_ID | HTTP_SAVE_CAPABILITY_ID => {
                let result = http::dispatch(&request).await?;
                network_egress_bytes = result.network_egress_bytes;
                result.output
            }
            SHELL_CAPABILITY_ID => {
                let (output, duration) = shell::dispatch(&request).await?;
                let wall_clock_ms = duration.as_millis().try_into().unwrap_or(u64::MAX);
                let output_bytes = bounded_output_bytes(&output, FIRST_PARTY_MAX_OUTPUT_BYTES)
                    .map_err(|error| {
                        error.with_usage(ResourceUsage {
                            wall_clock_ms,
                            network_egress_bytes,
                            process_count: 1,
                            ..ResourceUsage::default()
                        })
                    })?;
                return Ok(FirstPartyCapabilityResult::new(
                    output,
                    ResourceUsage {
                        wall_clock_ms,
                        output_bytes,
                        network_egress_bytes,
                        process_count: 1,
                        ..ResourceUsage::default()
                    },
                ));
            }
            SPAWN_SUBAGENT_CAPABILITY_ID => spawn_subagent::dispatch(),
            capability_id => {
                let Some(metadata) = coding_capability_metadata(capability_id) else {
                    return Err(FirstPartyCapabilityError::new(
                        RuntimeDispatchErrorKind::UndeclaredCapability,
                    ));
                };
                let request = CodingCapabilityRequest::new(
                    metadata.kind,
                    &request.scope,
                    request.mounts.as_ref(),
                    Arc::clone(&request.services.filesystem),
                    &request.input,
                );
                self.coding_state
                    .dispatch(&request)
                    .await
                    .map_err(coding_error)?
            }
        };
        let wall_clock_ms = start.elapsed().as_millis().try_into().unwrap_or(u64::MAX);
        let output_limit_bytes = match request.capability_id.as_str() {
            HTTP_CAPABILITY_ID | HTTP_SAVE_CAPABILITY_ID => http::MAX_HTTP_OUTPUT_BYTES,
            _ => FIRST_PARTY_MAX_OUTPUT_BYTES,
        };
        let output_bytes = bounded_output_bytes(&output, output_limit_bytes).map_err(|error| {
            if network_egress_bytes > 0 {
                error.with_usage(ResourceUsage {
                    wall_clock_ms,
                    network_egress_bytes,
                    ..ResourceUsage::default()
                })
            } else {
                error
            }
        })?;
        let usage = ResourceUsage {
            wall_clock_ms,
            output_bytes,
            network_egress_bytes,
            ..ResourceUsage::default()
        };
        Ok(FirstPartyCapabilityResult::new(output, usage))
    }
}

fn bounded_input_size(
    capability_id: &str,
    input: &serde_json::Value,
) -> Result<(), FirstPartyCapabilityError> {
    let bytes = serde_json::to_vec(input).map_err(|_| input_error())?;
    let max_bytes = coding_capability_metadata(capability_id)
        .map(|metadata| metadata.max_input_bytes)
        .unwrap_or(MAX_FIRST_PARTY_INPUT_BYTES);
    if bytes.len() > max_bytes {
        return Err(FirstPartyCapabilityError::new(
            RuntimeDispatchErrorKind::Resource,
        ));
    }
    Ok(())
}

fn bounded_output_bytes(
    output: &serde_json::Value,
    max_bytes: u64,
) -> Result<u64, FirstPartyCapabilityError> {
    let bytes = serde_json::to_vec(output).map_err(|_| input_error())?;
    let bytes = u64::try_from(bytes.len())
        .map_err(|_| FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::OutputTooLarge))?;
    if bytes > max_bytes {
        return Err(FirstPartyCapabilityError::new(
            RuntimeDispatchErrorKind::OutputTooLarge,
        ));
    }
    Ok(bytes)
}

/// Treat null sentinels as absent for declared optional fields.
///
/// Weaker models (notably quantized local models) routinely populate every
/// optional parameter with the string `"null"` instead of omitting it. Without
/// this normalization an optional `"null"` reaches a typed parser (e.g. an IANA
/// timezone) and aborts an otherwise valid call with `InputEncode`. Required
/// fields are left untouched so a legitimate `"null"` payload is preserved.
fn normalize_optional_null_sentinels(request: &mut FirstPartyCapabilityRequest) {
    let schema_name = request
        .capability_id
        .as_str()
        .strip_prefix("builtin.")
        .unwrap_or(request.capability_id.as_str())
        .replace('.', "-");
    let Some(schema) =
        resolve_builtin_input_schema_ref(&format!("schemas/builtin/{schema_name}.input.v1.json"))
    else {
        return;
    };
    let mut required: std::collections::HashSet<String> = schema
        .get("required")
        .and_then(|value| value.as_array())
        .map(|values| {
            values
                .iter()
                .filter_map(|value| value.as_str().map(ToString::to_string))
                .collect()
        })
        .unwrap_or_default();
    if let Some(branches) = schema.get("oneOf").and_then(|value| value.as_array()) {
        for branch in branches {
            if let Some(values) = branch.get("required").and_then(|value| value.as_array()) {
                required.extend(
                    values
                        .iter()
                        .filter_map(|value| value.as_str().map(ToString::to_string)),
                );
            }
        }
    }
    let declared: std::collections::HashSet<&str> = schema
        .get("properties")
        .and_then(|value| value.as_object())
        .map(|properties| properties.keys().map(String::as_str).collect())
        .unwrap_or_default();
    let Some(object) = request.input.as_object_mut() else {
        return;
    };
    object.retain(|key, value| {
        !(declared.contains(key.as_str())
            && !required.contains(key)
            && (value.as_str() == Some("null") || value.is_null()))
    });
}

fn resource_profile() -> Option<ResourceProfile> {
    Some(ResourceProfile {
        default_estimate: ResourceEstimate {
            wall_clock_ms: Some(FIRST_PARTY_DEFAULT_WALL_CLOCK_MS),
            output_bytes: Some(FIRST_PARTY_DEFAULT_OUTPUT_BYTES),
            ..ResourceEstimate::default()
        },
        hard_ceiling: Some(ResourceCeiling {
            max_usd: None,
            max_input_tokens: None,
            max_output_tokens: None,
            max_wall_clock_ms: Some(FIRST_PARTY_MAX_WALL_CLOCK_MS),
            max_output_bytes: Some(FIRST_PARTY_MAX_OUTPUT_BYTES),
            sandbox: None,
        }),
    })
}

fn input_error() -> FirstPartyCapabilityError {
    FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::InputEncode)
}

async fn run_egress_catching_panic<F, P>(
    future: F,
    panic_message: &'static str,
    on_panic: P,
) -> Result<Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError>, FirstPartyCapabilityError>
where
    F: Future<Output = Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError>>,
    P: FnOnce() -> FirstPartyCapabilityError,
{
    AssertUnwindSafe(future).catch_unwind().await.map_err(|_| {
        tracing::error!("{panic_message}");
        on_panic()
    })
}

fn operation_error() -> FirstPartyCapabilityError {
    FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::OperationFailed)
}

fn coding_error(error: CodingCapabilityError) -> FirstPartyCapabilityError {
    FirstPartyCapabilityError::new(error.kind())
}

fn coding_capability_metadata(capability_id: &str) -> Option<CodingCapabilityMetadata> {
    CODING_CAPABILITIES
        .iter()
        .copied()
        .find(|metadata| metadata.id == capability_id)
}
