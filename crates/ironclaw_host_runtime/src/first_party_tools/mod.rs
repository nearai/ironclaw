//! Built-in first-party capability handlers.
//!
//! These are host-owned capabilities, not extension-declared tools. They keep
//! pure tool logic behind the Reborn capability path so callers still pass
//! through `CapabilityHost`, trust policy, grants, resource accounting, and
//! runtime dispatch before any handler runs.

mod echo;
mod http;
mod http_output;
mod json;
mod memory;
mod model_visible_output;
mod profile_set;
mod schemas;
mod shell;
mod skill_management;
mod skill_url_install;
mod spawn_subagent;
mod time;
mod trace_commons;
mod trigger_management;

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
    PermissionMode, ProcessBackendKind, RequestedTrustClass, ResourceCeiling, ResourceEstimate,
    ResourceProfile, ResourceUsage, RuntimeDispatchErrorKind, RuntimeHttpEgressError,
    RuntimeHttpEgressResponse, TrustClass, VirtualPath,
};

use crate::{
    FirstPartyCapabilityError, FirstPartyCapabilityHandler, FirstPartyCapabilityRegistry,
    FirstPartyCapabilityRequest, FirstPartyCapabilityResult,
};

pub(crate) use self::schemas::resolve_builtin_input_schema_ref;

pub use echo::ECHO_CAPABILITY_ID;
pub use http::{HTTP_CAPABILITY_ID, HTTP_SAVE_CAPABILITY_ID};
pub use json::JSON_CAPABILITY_ID;
pub use memory::{
    MEMORY_READ_CAPABILITY_ID, MEMORY_SEARCH_CAPABILITY_ID, MEMORY_TREE_CAPABILITY_ID,
    MEMORY_WRITE_CAPABILITY_ID,
};
pub use profile_set::PROFILE_SET_CAPABILITY_ID;
pub use shell::SHELL_CAPABILITY_ID;
pub use skill_management::{
    SKILL_INSTALL_CAPABILITY_ID, SKILL_LIST_CAPABILITY_ID, SKILL_REMOVE_CAPABILITY_ID,
};
pub use spawn_subagent::SPAWN_SUBAGENT_CAPABILITY_ID;
pub use time::TIME_CAPABILITY_ID;
pub use trace_commons::{
    TRACE_COMMONS_ACCOUNT_LOGIN_LINK_CAPABILITY_ID, TRACE_COMMONS_CREDITS_CAPABILITY_ID,
    TRACE_COMMONS_ONBOARD_CAPABILITY_ID, TRACE_COMMONS_PROFILE_SET_CAPABILITY_ID,
    TRACE_COMMONS_PROFILE_TOKEN_CAPABILITY_ID, TRACE_COMMONS_STATUS_CAPABILITY_ID,
};
#[cfg(any(test, feature = "test-support"))]
pub use trigger_management::TriggerManagementClock;
pub use trigger_management::{
    TRIGGER_CREATE_CAPABILITY_ID, TRIGGER_LIST_CAPABILITY_ID, TRIGGER_PAUSE_CAPABILITY_ID,
    TRIGGER_REMOVE_CAPABILITY_ID, TRIGGER_RESUME_CAPABILITY_ID, TriggerCreateHook,
};

pub const BUILTIN_FIRST_PARTY_PROVIDER: &str = "builtin";
pub const READ_FILE_CAPABILITY_ID: &str = "builtin.read_file";
pub const WRITE_FILE_CAPABILITY_ID: &str = "builtin.write_file";
pub const LIST_DIR_CAPABILITY_ID: &str = "builtin.list_dir";
pub const GLOB_CAPABILITY_ID: &str = "builtin.glob";
pub const GREP_CAPABILITY_ID: &str = "builtin.grep";
pub const APPLY_PATCH_CAPABILITY_ID: &str = "builtin.apply_patch";

// `builtin.shell` is the only built-in first-party handler that directly
// requires a RuntimeProcessPort. `builtin.spawn_subagent` declares
// SpawnProcess as an authorization effect, but child-run scheduling is governed
// by runtime-policy planning rather than this process-port capability list.
const PROCESS_PORT_BACKED_BUILTIN_CAPABILITY_IDS: &[&str] = &[SHELL_CAPABILITY_ID];

const MAX_FIRST_PARTY_INPUT_BYTES: usize = 1_048_576;
const MAX_WRITE_FILE_INPUT_BYTES: usize = 6 * 1024 * 1024;
const MAX_APPLY_PATCH_INPUT_BYTES: usize = 21 * 1024 * 1024;
const FIRST_PARTY_DEFAULT_OUTPUT_BYTES: u64 = 16 * 1024;
pub(super) const FIRST_PARTY_MAX_OUTPUT_BYTES: u64 = 1_048_576;
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
        description: "Read text files, and extract text from supported document files, through scoped mounts with v1 read_file output shape",
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
                    trace_commons::onboard_manifest()?,
                    trace_commons::status_manifest()?,
                    trace_commons::credits_manifest()?,
                    trace_commons::profile_token_manifest()?,
                    trace_commons::profile_set_manifest()?,
                    trace_commons::account_login_link_manifest()?,
                    profile_set::manifest()?,
                ];
                capabilities.extend(memory::manifests()?);
                capabilities.extend(coding_manifests()?);
                capabilities.extend(skill_management::manifests()?);
                capabilities.extend(trigger_management::manifests()?);
                capabilities
            },
            // The built-in first-party package declares no manifest hooks;
            // first-party builtin hooks are installed by the composition
            // loader directly, not via this manifest surface.
            hooks: Vec::new(),
        },
        VirtualPath::new("/system/extensions/builtin")?,
    )
}

pub fn builtin_first_party_package_for_process_backend(
    process_backend: ProcessBackendKind,
) -> Result<ExtensionPackage, ExtensionError> {
    let mut package = builtin_first_party_package()?;
    if !process_port_backed_builtins_enabled(process_backend) {
        remove_process_port_backed_builtin_capabilities(&mut package)?;
    }
    Ok(package)
}

fn process_port_backed_builtins_enabled(process_backend: ProcessBackendKind) -> bool {
    matches!(
        process_backend,
        ProcessBackendKind::Docker
            | ProcessBackendKind::Srt
            | ProcessBackendKind::SmolVm
            | ProcessBackendKind::LocalHost
            | ProcessBackendKind::TenantSandbox
            | ProcessBackendKind::OrgDedicatedRunner
    )
}

fn remove_process_port_backed_builtin_capabilities(
    package: &mut ExtensionPackage,
) -> Result<(), ExtensionError> {
    for capability_id in PROCESS_PORT_BACKED_BUILTIN_CAPABILITY_IDS {
        remove_builtin_capability(package, capability_id)?;
    }
    Ok(())
}

fn remove_builtin_capability(
    package: &mut ExtensionPackage,
    capability_id: &str,
) -> Result<(), ExtensionError> {
    let capability_id = CapabilityId::new(capability_id)?;
    let descriptor_present = package
        .capabilities
        .iter()
        .any(|candidate| candidate.id == capability_id);
    let manifest_present = package
        .manifest
        .capabilities
        .iter()
        .any(|candidate| candidate.id == capability_id);
    if !descriptor_present || !manifest_present {
        return Err(ExtensionError::InvalidManifest {
            reason: format!(
                "built-in first-party package is missing process-port-backed capability {capability_id}"
            ),
        });
    }

    package
        .capabilities
        .retain(|candidate| candidate.id != capability_id);
    package
        .manifest
        .capabilities
        .retain(|candidate| candidate.id != capability_id);
    Ok(())
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

/// Create handlers for all built-in first-party capabilities using an
/// explicitly composed trigger repository.
pub fn builtin_first_party_handlers(
    trigger_repository: Arc<dyn ironclaw_triggers::TriggerRepository>,
) -> Result<FirstPartyCapabilityRegistry, HostApiError> {
    let mut registry = builtin_first_party_base_registry()?;
    trigger_management::insert_handlers(&mut registry, trigger_repository)?;
    Ok(registry)
}

pub fn builtin_first_party_handlers_for_process_backend(
    trigger_repository: Arc<dyn ironclaw_triggers::TriggerRepository>,
    process_backend: ProcessBackendKind,
) -> Result<FirstPartyCapabilityRegistry, HostApiError> {
    let mut registry = builtin_first_party_handlers(trigger_repository)?;
    if !process_port_backed_builtins_enabled(process_backend) {
        remove_process_port_backed_builtin_handlers(&mut registry)?;
    }
    Ok(registry)
}

/// Create handlers for all built-in first-party capabilities using an
/// explicitly composed trigger repository and trigger-create lifecycle hook.
pub fn builtin_first_party_handlers_with_trigger_create_hook(
    trigger_repository: Arc<dyn ironclaw_triggers::TriggerRepository>,
    trigger_create_hook: Arc<dyn TriggerCreateHook>,
) -> Result<FirstPartyCapabilityRegistry, HostApiError> {
    let mut registry = builtin_first_party_base_registry()?;
    trigger_management::insert_handlers_with_create_hook(
        &mut registry,
        trigger_repository,
        trigger_create_hook,
    )?;
    Ok(registry)
}

pub fn builtin_first_party_handlers_with_trigger_create_hook_for_process_backend(
    trigger_repository: Arc<dyn ironclaw_triggers::TriggerRepository>,
    trigger_create_hook: Arc<dyn TriggerCreateHook>,
    process_backend: ProcessBackendKind,
) -> Result<FirstPartyCapabilityRegistry, HostApiError> {
    let mut registry = builtin_first_party_handlers_with_trigger_create_hook(
        trigger_repository,
        trigger_create_hook,
    )?;
    if !process_port_backed_builtins_enabled(process_backend) {
        remove_process_port_backed_builtin_handlers(&mut registry)?;
    }
    Ok(registry)
}

fn remove_process_port_backed_builtin_handlers(
    registry: &mut FirstPartyCapabilityRegistry,
) -> Result<(), HostApiError> {
    for capability_id in PROCESS_PORT_BACKED_BUILTIN_CAPABILITY_IDS {
        remove_builtin_handler(registry, capability_id)?;
    }
    Ok(())
}

fn remove_builtin_handler(
    registry: &mut FirstPartyCapabilityRegistry,
    capability_id: &str,
) -> Result<(), HostApiError> {
    let capability_id = CapabilityId::new(capability_id)?;
    if !registry.contains_handler(&capability_id) {
        return Err(HostApiError::InvariantViolation {
            reason: format!(
                "built-in first-party handlers are missing process-port-backed capability {capability_id}"
            ),
        });
    }
    registry.remove_handler(&capability_id);
    Ok(())
}

#[cfg(any(test, feature = "test-support"))]
#[doc(hidden)]
pub fn builtin_first_party_handlers_with_trigger_clock(
    trigger_repository: Arc<dyn ironclaw_triggers::TriggerRepository>,
    trigger_clock: Arc<dyn TriggerManagementClock>,
) -> Result<FirstPartyCapabilityRegistry, HostApiError> {
    let mut registry = builtin_first_party_base_registry()?;
    trigger_management::insert_handlers_with_clock(
        &mut registry,
        trigger_repository,
        trigger_clock,
    )?;
    Ok(registry)
}

fn builtin_first_party_base_registry() -> Result<FirstPartyCapabilityRegistry, HostApiError> {
    let handler = Arc::new(BuiltinFirstPartyTools::default());
    let mut registry = FirstPartyCapabilityRegistry::new()
        .with_handler(CapabilityId::new(ECHO_CAPABILITY_ID)?, handler.clone())
        .with_handler(CapabilityId::new(TIME_CAPABILITY_ID)?, handler.clone())
        .with_handler(CapabilityId::new(JSON_CAPABILITY_ID)?, handler.clone())
        .with_handler(CapabilityId::new(HTTP_CAPABILITY_ID)?, handler.clone())
        .with_handler(CapabilityId::new(HTTP_SAVE_CAPABILITY_ID)?, handler.clone())
        .with_handler(
            CapabilityId::new(MEMORY_SEARCH_CAPABILITY_ID)?,
            handler.clone(),
        )
        .with_handler(
            CapabilityId::new(MEMORY_WRITE_CAPABILITY_ID)?,
            handler.clone(),
        )
        .with_handler(
            CapabilityId::new(MEMORY_READ_CAPABILITY_ID)?,
            handler.clone(),
        )
        .with_handler(
            CapabilityId::new(MEMORY_TREE_CAPABILITY_ID)?,
            handler.clone(),
        )
        .with_handler(CapabilityId::new(SHELL_CAPABILITY_ID)?, handler.clone());
    for metadata in CODING_CAPABILITIES {
        registry.insert_handler(CapabilityId::new(metadata.id)?, handler.clone());
    }
    registry.insert_handler(
        CapabilityId::new(SPAWN_SUBAGENT_CAPABILITY_ID)?,
        handler.clone(),
    );
    registry.insert_handler(
        CapabilityId::new(TRACE_COMMONS_ONBOARD_CAPABILITY_ID)?,
        handler.clone(),
    );
    registry.insert_handler(
        CapabilityId::new(TRACE_COMMONS_STATUS_CAPABILITY_ID)?,
        handler.clone(),
    );
    registry.insert_handler(
        CapabilityId::new(TRACE_COMMONS_CREDITS_CAPABILITY_ID)?,
        handler.clone(),
    );
    registry.insert_handler(
        CapabilityId::new(TRACE_COMMONS_PROFILE_TOKEN_CAPABILITY_ID)?,
        handler.clone(),
    );
    registry.insert_handler(
        CapabilityId::new(TRACE_COMMONS_PROFILE_SET_CAPABILITY_ID)?,
        handler.clone(),
    );
    registry.insert_handler(
        CapabilityId::new(TRACE_COMMONS_ACCOUNT_LOGIN_LINK_CAPABILITY_ID)?,
        handler.clone(),
    );
    registry.insert_handler(CapabilityId::new(PROFILE_SET_CAPABILITY_ID)?, handler);
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
    memory_state: memory::MemoryCapabilityState,
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
        let (output, display_preview) = match request.capability_id.as_str() {
            ECHO_CAPABILITY_ID => (echo::dispatch(&request.input)?, None),
            TIME_CAPABILITY_ID => (time::dispatch(&request.input)?, None),
            JSON_CAPABILITY_ID => (json::dispatch(&request.input)?, None),
            HTTP_CAPABILITY_ID | HTTP_SAVE_CAPABILITY_ID => {
                let result = http::dispatch(&request).await?;
                network_egress_bytes = result.network_egress_bytes;
                (result.output, None)
            }
            MEMORY_SEARCH_CAPABILITY_ID
            | MEMORY_WRITE_CAPABILITY_ID
            | MEMORY_READ_CAPABILITY_ID
            | MEMORY_TREE_CAPABILITY_ID => {
                let mut result = memory::dispatch(&self.memory_state, &request).await?;
                result.usage.output_bytes =
                    bounded_output_bytes(&result.output, FIRST_PARTY_MAX_OUTPUT_BYTES)?;
                return Ok(result);
            }
            PROFILE_SET_CAPABILITY_ID => {
                let mut result = profile_set::dispatch(&self.memory_state, &request).await?;
                result.usage.output_bytes =
                    bounded_output_bytes(&result.output, FIRST_PARTY_MAX_OUTPUT_BYTES)?;
                result.usage.wall_clock_ms =
                    start.elapsed().as_millis().try_into().unwrap_or(u64::MAX);
                return Ok(result);
            }
            SHELL_CAPABILITY_ID => {
                let (output, duration) = shell::dispatch(&request).await?;
                let wall_clock_ms = duration.as_millis().try_into().unwrap_or(u64::MAX);
                let output_bytes = bounded_output_bytes(&output, FIRST_PARTY_MAX_OUTPUT_BYTES)
                    .map_err(|error| {
                        error.with_usage(
                            ResourceUsage::default()
                                .set_wall_clock_ms(wall_clock_ms)
                                .set_network_egress_bytes(network_egress_bytes)
                                .set_process_count(1),
                        )
                    })?;
                return Ok(FirstPartyCapabilityResult::new(
                    output,
                    ResourceUsage::default()
                        .set_wall_clock_ms(wall_clock_ms)
                        .set_output_bytes(output_bytes)
                        .set_network_egress_bytes(network_egress_bytes)
                        .set_process_count(1),
                ));
            }
            SPAWN_SUBAGENT_CAPABILITY_ID => (spawn_subagent::dispatch(), None),
            // arch-exempt: network_egress_bytes not surfaced for the onboard
            // call — it routes through the host runtime_http_egress (policy- and
            // credential-checked), but dispatch_onboard returns only the output
            // Value, so outbound byte accounting is not propagated back here.
            // Low-frequency, consent-gated onboarding call.
            TRACE_COMMONS_ONBOARD_CAPABILITY_ID => {
                (trace_commons::dispatch_onboard(&request).await?, None)
            }
            TRACE_COMMONS_STATUS_CAPABILITY_ID => {
                (trace_commons::dispatch_status(&request).await?, None)
            }
            TRACE_COMMONS_CREDITS_CAPABILITY_ID => {
                (trace_commons::dispatch_credits(&request).await?, None)
            }
            TRACE_COMMONS_PROFILE_TOKEN_CAPABILITY_ID => {
                (trace_commons::dispatch_profile_token(&request).await?, None)
            }
            TRACE_COMMONS_PROFILE_SET_CAPABILITY_ID => {
                (trace_commons::dispatch_profile_set(&request).await?, None)
            }
            TRACE_COMMONS_ACCOUNT_LOGIN_LINK_CAPABILITY_ID => (
                trace_commons::dispatch_account_login_link(&request).await?,
                None,
            ),
            capability_id => {
                let Some(metadata) = coding_capability_metadata(capability_id) else {
                    return Err(FirstPartyCapabilityError::new(
                        RuntimeDispatchErrorKind::UndeclaredCapability,
                    ));
                };
                let request = CodingCapabilityRequest::new(
                    &request.capability_id,
                    metadata.kind,
                    &request.scope,
                    request.mounts.as_ref(),
                    Arc::clone(&request.services.filesystem),
                    &request.input,
                );
                let result = self
                    .coding_state
                    .dispatch(&request)
                    .await
                    .map_err(coding_error)?;
                (result.output, result.display_preview)
            }
        };
        let wall_clock_ms = start.elapsed().as_millis().try_into().unwrap_or(u64::MAX);
        let output_limit_bytes = match request.capability_id.as_str() {
            HTTP_CAPABILITY_ID | HTTP_SAVE_CAPABILITY_ID => http::MAX_HTTP_OUTPUT_BYTES,
            _ => FIRST_PARTY_MAX_OUTPUT_BYTES,
        };
        let output_bytes = bounded_output_bytes(&output, output_limit_bytes).map_err(|error| {
            if network_egress_bytes > 0 {
                error.with_usage(
                    ResourceUsage::default()
                        .set_wall_clock_ms(wall_clock_ms)
                        .set_network_egress_bytes(network_egress_bytes),
                )
            } else {
                error
            }
        })?;
        let usage = ResourceUsage::default()
            .set_wall_clock_ms(wall_clock_ms)
            .set_output_bytes(output_bytes)
            .set_network_egress_bytes(network_egress_bytes);
        Ok(FirstPartyCapabilityResult::new(output, usage).with_display_preview(display_preview))
    }
}

pub(super) fn bounded_input_size(
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

pub(super) fn bounded_output_bytes(
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
        default_estimate: ResourceEstimate::default()
            .set_wall_clock_ms(FIRST_PARTY_DEFAULT_WALL_CLOCK_MS)
            .set_output_bytes(FIRST_PARTY_DEFAULT_OUTPUT_BYTES),
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
    match error.safe_summary() {
        Some(summary) => FirstPartyCapabilityError::with_safe_summary(error.kind(), summary),
        None => FirstPartyCapabilityError::new(error.kind()),
    }
}

fn coding_capability_metadata(capability_id: &str) -> Option<CodingCapabilityMetadata> {
    CODING_CAPABILITIES
        .iter()
        .copied()
        .find(|metadata| metadata.id == capability_id)
}
