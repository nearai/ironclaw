//! Built-in first-party capability handlers.
//!
//! These are host-owned capabilities, not extension-declared tools. They keep
//! pure tool logic behind the Reborn capability path so callers still pass
//! through `CapabilityHost`, trust policy, grants, resource accounting, and
//! runtime dispatch before any handler runs.

mod coding;
mod echo;
mod http;
mod json;
mod shell;
mod skill_management;
mod time;

use std::{sync::Arc, time::Instant};

use async_trait::async_trait;
use ironclaw_extensions::{
    CapabilityManifest, CapabilityVisibility, ExtensionError, ExtensionManifest, ExtensionPackage,
    ExtensionRuntime, MANIFEST_SCHEMA_VERSION, ManifestSource,
};
use ironclaw_host_api::{
    CapabilityId, CapabilityProfileSchemaRef, EffectKind, ExtensionId, HostApiError,
    PermissionMode, RequestedTrustClass, ResourceCeiling, ResourceEstimate, ResourceProfile,
    ResourceUsage, RuntimeDispatchErrorKind, TrustClass, VirtualPath,
};

use crate::{
    FirstPartyCapabilityError, FirstPartyCapabilityHandler, FirstPartyCapabilityRegistry,
    FirstPartyCapabilityRequest, FirstPartyCapabilityResult,
};

pub use coding::{
    APPLY_PATCH_CAPABILITY_ID, GLOB_CAPABILITY_ID, GREP_CAPABILITY_ID, LIST_DIR_CAPABILITY_ID,
    READ_FILE_CAPABILITY_ID, WRITE_FILE_CAPABILITY_ID,
};
pub use echo::ECHO_CAPABILITY_ID;
pub use http::HTTP_CAPABILITY_ID;
pub use json::JSON_CAPABILITY_ID;
pub use shell::SHELL_CAPABILITY_ID;
pub use skill_management::{
    SKILL_INSTALL_CAPABILITY_ID, SKILL_LIST_CAPABILITY_ID, SKILL_REMOVE_CAPABILITY_ID,
};
pub use time::TIME_CAPABILITY_ID;

pub const BUILTIN_FIRST_PARTY_PROVIDER: &str = "builtin";

const MAX_FIRST_PARTY_INPUT_BYTES: usize = 1_048_576;
const MAX_WRITE_FILE_INPUT_BYTES: usize = 6 * 1024 * 1024;
const MAX_APPLY_PATCH_INPUT_BYTES: usize = 21 * 1024 * 1024;
const FIRST_PARTY_DEFAULT_OUTPUT_BYTES: u64 = 16 * 1024;
const FIRST_PARTY_MAX_OUTPUT_BYTES: u64 = 1_048_576;
const FIRST_PARTY_DEFAULT_WALL_CLOCK_MS: u64 = 100;
const FIRST_PARTY_MAX_WALL_CLOCK_MS: u64 = 5_000;

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
                    shell::manifest()?,
                ];
                capabilities.extend(coding::manifests()?);
                capabilities.extend(skill_management::manifests()?);
                capabilities
            },
        },
        VirtualPath::new("/system/extensions/builtin")?,
    )
}

/// Create handlers for all built-in first-party capabilities.
pub fn builtin_first_party_handlers() -> Result<FirstPartyCapabilityRegistry, HostApiError> {
    let handler = Arc::new(BuiltinFirstPartyTools::default());
    let mut registry = FirstPartyCapabilityRegistry::new()
        .with_handler(CapabilityId::new(ECHO_CAPABILITY_ID)?, handler.clone())
        .with_handler(CapabilityId::new(TIME_CAPABILITY_ID)?, handler.clone())
        .with_handler(CapabilityId::new(JSON_CAPABILITY_ID)?, handler.clone())
        .with_handler(CapabilityId::new(HTTP_CAPABILITY_ID)?, handler.clone())
        .with_handler(CapabilityId::new(SHELL_CAPABILITY_ID)?, handler.clone())
        .with_handler(CapabilityId::new(READ_FILE_CAPABILITY_ID)?, handler.clone())
        .with_handler(
            CapabilityId::new(WRITE_FILE_CAPABILITY_ID)?,
            handler.clone(),
        )
        .with_handler(CapabilityId::new(LIST_DIR_CAPABILITY_ID)?, handler.clone())
        .with_handler(CapabilityId::new(GLOB_CAPABILITY_ID)?, handler.clone())
        .with_handler(CapabilityId::new(GREP_CAPABILITY_ID)?, handler.clone())
        .with_handler(CapabilityId::new(APPLY_PATCH_CAPABILITY_ID)?, handler);
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
        prompt_doc_ref: Some(CapabilityProfileSchemaRef::new(format!(
            "prompts/builtin/{schema_name}.md"
        ))?),
        required_host_ports: Vec::new(),
        resource_profile,
    })
}

#[derive(Debug, Default)]
pub struct BuiltinFirstPartyTools {
    coding_read_state: coding::SharedCodingReadState,
    coding_edit_locks: coding::SharedCodingEditLocks,
}

#[async_trait]
impl FirstPartyCapabilityHandler for BuiltinFirstPartyTools {
    async fn dispatch(
        &self,
        request: FirstPartyCapabilityRequest,
    ) -> Result<FirstPartyCapabilityResult, FirstPartyCapabilityError> {
        bounded_input_size(request.capability_id.as_str(), &request.input)?;
        let start = Instant::now();
        let mut network_egress_bytes = 0;
        let output = match request.capability_id.as_str() {
            ECHO_CAPABILITY_ID => echo::dispatch(&request.input)?,
            TIME_CAPABILITY_ID => time::dispatch(&request.input)?,
            JSON_CAPABILITY_ID => json::dispatch(&request.input)?,
            HTTP_CAPABILITY_ID => {
                let result = http::dispatch(&request).await?;
                network_egress_bytes = result.network_egress_bytes;
                result.output
            }
            SHELL_CAPABILITY_ID => {
                let (output, duration) = shell::dispatch(&request).await?;
                let wall_clock_ms = duration.as_millis().try_into().unwrap_or(u64::MAX);
                let output_bytes = bounded_output_bytes(&output).map_err(|error| {
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
            READ_FILE_CAPABILITY_ID
            | WRITE_FILE_CAPABILITY_ID
            | LIST_DIR_CAPABILITY_ID
            | GLOB_CAPABILITY_ID
            | GREP_CAPABILITY_ID
            | APPLY_PATCH_CAPABILITY_ID => {
                coding::dispatch(&request, &self.coding_read_state, &self.coding_edit_locks).await?
            }
            _ => {
                return Err(FirstPartyCapabilityError::new(
                    RuntimeDispatchErrorKind::UndeclaredCapability,
                ));
            }
        };
        let wall_clock_ms = start.elapsed().as_millis().try_into().unwrap_or(u64::MAX);
        let output_bytes = bounded_output_bytes(&output).map_err(|error| {
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
    let max_bytes = match capability_id {
        WRITE_FILE_CAPABILITY_ID => MAX_WRITE_FILE_INPUT_BYTES,
        APPLY_PATCH_CAPABILITY_ID => MAX_APPLY_PATCH_INPUT_BYTES,
        _ => MAX_FIRST_PARTY_INPUT_BYTES,
    };
    if bytes.len() > max_bytes {
        return Err(FirstPartyCapabilityError::new(
            RuntimeDispatchErrorKind::Resource,
        ));
    }
    Ok(())
}

fn bounded_output_bytes(output: &serde_json::Value) -> Result<u64, FirstPartyCapabilityError> {
    let bytes = serde_json::to_vec(output).map_err(|_| input_error())?;
    let bytes = u64::try_from(bytes.len())
        .map_err(|_| FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::OutputTooLarge))?;
    if bytes > FIRST_PARTY_MAX_OUTPUT_BYTES {
        return Err(FirstPartyCapabilityError::new(
            RuntimeDispatchErrorKind::OutputTooLarge,
        ));
    }
    Ok(bytes)
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

fn guest_error() -> FirstPartyCapabilityError {
    FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::Guest)
}
