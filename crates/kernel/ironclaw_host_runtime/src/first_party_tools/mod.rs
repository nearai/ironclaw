//! Built-in first-party capability handlers.
//!
//! These are host-owned capabilities, not extension-declared tools. They keep
//! pure tool logic behind the Reborn capability path so callers still pass
//! through `CapabilityHost`, trust policy, grants, resource accounting, and
//! runtime dispatch before any handler runs.

mod coding;
mod echo;
mod http;
mod http_output;
mod json;
mod memory;
mod model_visible_output;
mod outbound_deliver;
mod reply_attachment;
mod schemas;
mod shell;
mod skill_management;
mod spawn_subagent;
mod time;
mod trace_commons;
mod trigger_creation;
mod trigger_management;

use std::{future::Future, panic::AssertUnwindSafe, sync::Arc, time::Instant};

use async_trait::async_trait;
use futures_util::FutureExt as _;
use ironclaw_extension_contracts::runtime::ExtensionRuntime;
use ironclaw_extension_registry::{
    CapabilityManifest, CapabilityVisibility, ExtensionError, ExtensionManifest, ExtensionPackage,
    MANIFEST_SCHEMA_VERSION, ManifestSource,
};
use ironclaw_host_api::{
    capability::{EffectKind, OriginGateMatrix, OriginGatePolicy, PermissionMode},
    capability_profile::CapabilityProfileSchemaRef,
    dispatch::RuntimeDispatchErrorKind,
    error::HostApiError,
    http::{RuntimeHttpEgressError, RuntimeHttpEgressResponse},
    ids::{CapabilityId, ExtensionId},
    path::VirtualPath,
    resource::{ResourceCeiling, ResourceEstimate, ResourceProfile, ResourceUsage},
    runtime::TrustClass,
    runtime_policy::ProcessBackendKind,
    trust::RequestedTrustClass,
};

use crate::{
    FirstPartyCapabilityError, FirstPartyCapabilityHandler, FirstPartyCapabilityRegistry,
    FirstPartyCapabilityRequest, FirstPartyCapabilityResult,
};

pub(crate) use self::schemas::{
    resolve_builtin_input_schema_ref, resolve_native_memory_input_schema_ref,
};

use coding::coding_manifests;
pub use coding::{
    CODING_BASH_CAPABILITY_ID, CODING_EDIT_CAPABILITY_ID, CODING_GLOB_CAPABILITY_ID,
    CODING_GREP_CAPABILITY_ID, CODING_READ_CAPABILITY_ID, CODING_WRITE_CAPABILITY_ID, CodingTools,
    DOCUMENT_EDIT_CAPABILITY_ID, HTML_TO_PDF_CAPABILITY_ID, coding_package, insert_coding_handlers,
};
pub use echo::ECHO_CAPABILITY_ID;
pub use http::{HTTP_CAPABILITY_ID, HTTP_SAVE_CAPABILITY_ID};
pub use ironclaw_memory::{
    MEMORY_READ_CAPABILITY_ID, MEMORY_SEARCH_CAPABILITY_ID, MEMORY_TREE_CAPABILITY_ID,
    MEMORY_WRITE_CAPABILITY_ID, PROFILE_SET_CAPABILITY_ID,
};
pub use json::JSON_CAPABILITY_ID;
pub use memory::{
    MemoryToolProfile, NativeMemoryToolHandler, ensure_memory_mount, finish_memory_tool_result,
    invocation_for_request as memory_invocation_for_request, map_memory_service_error,
    memory_tool_profiles, normalize_memory_tool_input, register_memory_tool_handler,
    register_native_memory_tools,
};
pub use outbound_deliver::OUTBOUND_DELIVER_CAPABILITY_ID;
pub use reply_attachment::ATTACH_WORKSPACE_FILE_TO_REPLY_CAPABILITY_ID;
pub use shell::SHELL_CAPABILITY_ID;
pub use skill_management::{
    SKILL_AUTO_ACTIVATE_SET_CAPABILITY_ID, SKILL_INSTALL_CAPABILITY_ID, SKILL_LIST_CAPABILITY_ID,
    SKILL_REMOVE_CAPABILITY_ID, SKILL_UPDATE_CAPABILITY_ID,
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
    TRIGGER_REMOVE_CAPABILITY_ID, TRIGGER_RESUME_CAPABILITY_ID, TRIGGER_RUN_CAPABILITY_ID,
    TriggerCreateHook,
};

pub const BUILTIN_FIRST_PARTY_PROVIDER: &str = "builtin";

/// Provider id of the always-on native memory extension. It rides the same
/// host-bundled, always-on first-party lane as `builtin` (not the
/// catalog/lifecycle extension lane), so its model-facing memory tools are
/// unconditionally available — preserving the former `builtin.memory_*`
/// behavior. Provider-swapping stays on the compose-time memory binding.
///
/// Aliases the canonical id owned by [`crate::memory_native_extension`] (which
/// also owns the bundled manifests + the registrable package builders), so the
/// surface/trust seams here and the binding layer share one identity string.
pub const NATIVE_MEMORY_FIRST_PARTY_PROVIDER: &str =
    crate::memory_native_extension::NATIVE_MEMORY_EXTENSION_ID;

/// The registry-lane provider allowlist once activated extension dispatch
/// resolves from the extension host's active snapshot: only the always-on
/// registry-lane packages — the synthetic built-in package and the BOUND
/// memory provider's package — keep resolving through the registry. None of
/// them is ever published in the extension host's active snapshot, so
/// omitting one here makes its capabilities unresolvable
/// (`UnknownCapability`) in every composition that installs an extension
/// host. Every bundled memory provider id is listed (only the bound one has
/// a registered package, so the others stay inert).
pub(crate) fn builtin_provider_allowlist() -> std::collections::BTreeSet<ExtensionId> {
    let mut allowlist = std::collections::BTreeSet::new();
    if let Ok(builtin) = ExtensionId::new(BUILTIN_FIRST_PARTY_PROVIDER) {
        allowlist.insert(builtin);
    }
    for provider in crate::memory_native_extension::MEMORY_PROVIDER_PACKAGE_IDS {
        if let Ok(memory) = ExtensionId::new(*provider) {
            allowlist.insert(memory);
        }
    }
    allowlist
}
// Canonical capability ids of the pinned `glob` and `grep` engines
// (`coding.rs` aliases these as `CODING_GLOB_CAPABILITY_ID` /
// `CODING_GREP_CAPABILITY_ID`). The
// v1 coding ids they replaced (`builtin.read_file`, `builtin.write_file`,
// `builtin.list_dir`, `builtin.apply_patch`) are retired.
pub const GLOB_CAPABILITY_ID: &str = "builtin.glob";
pub const GREP_CAPABILITY_ID: &str = "builtin.grep";

// `builtin.shell` and the pinned `builtin.bash` are the built-in first-party
// handlers that directly require a RuntimeProcessPort. `builtin.spawn_subagent`
// declares SpawnProcess as an authorization effect, but child-run scheduling is
// governed by runtime-policy planning rather than this process-port capability list.
const PROCESS_PORT_BACKED_BUILTIN_CAPABILITY_IDS: &[&str] =
    &[SHELL_CAPABILITY_ID, coding::CODING_BASH_CAPABILITY_ID];

const MAX_FIRST_PARTY_INPUT_BYTES: usize = 1_048_576;
const MAX_WRITE_FILE_INPUT_BYTES: usize = 6 * 1024 * 1024;
const FIRST_PARTY_DEFAULT_OUTPUT_BYTES: u64 = 16 * 1024;
pub(super) const FIRST_PARTY_MAX_OUTPUT_BYTES: u64 = 1_048_576;
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
            host_api_surfaces: Vec::new(),
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
                    outbound_deliver::manifest()?,
                    reply_attachment::manifest()?,
                ];
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
    // Process restrictions are applied after assembling the canonical
    // coding-first builtin package.
    coding_package(process_backend)
}

fn restrict_package_for_process_backend(
    package: &mut ExtensionPackage,
    process_backend: ProcessBackendKind,
) -> Result<(), ExtensionError> {
    if !process_port_backed_builtins_enabled(process_backend) {
        remove_process_port_backed_builtin_capabilities(package)?;
    } else if process_backend == ProcessBackendKind::UserSandbox {
        // The PR1 user sandbox owns its isolated `/workspace` and runs with
        // direct container networking but no host network service. These are
        // not host-filesystem or host-network effects, so do not ask the
        // invocation resolver to bind either service. Brokered network effects
        // remain a follow-up once process traffic can traverse ironclaw_network.
        append_user_sandbox_process_guidance(package)?;
        for effect in [
            EffectKind::ReadFilesystem,
            EffectKind::WriteFilesystem,
            EffectKind::Network,
        ] {
            remove_builtin_capability_effect(package, SHELL_CAPABILITY_ID, effect)?;
            remove_builtin_capability_effect(package, coding::CODING_BASH_CAPABILITY_ID, effect)?;
        }
    }
    Ok(())
}

fn append_user_sandbox_process_guidance(
    package: &mut ExtensionPackage,
) -> Result<(), ExtensionError> {
    const GUIDANCE: &str = " Runs inside a per-user sandbox with a writable persistent `/workspace` and a read-only system filesystem. Install Python packages under `/workspace`, preferably with `python3 -m venv /workspace/.venv`, then use `/workspace/.venv/bin/python` and `/workspace/.venv/bin/pip` in later calls because shell process state does not persist between calls.";

    for capability_id in [SHELL_CAPABILITY_ID, coding::CODING_BASH_CAPABILITY_ID] {
        let capability_id = CapabilityId::new(capability_id)?;
        let descriptor = package
            .capabilities
            .iter_mut()
            .find(|candidate| candidate.id == capability_id)
            .ok_or_else(|| ExtensionError::InvalidManifest {
                reason: format!(
                    "built-in first-party package is missing capability {capability_id}"
                ),
            })?;
        let manifest = package
            .manifest
            .capabilities
            .iter_mut()
            .find(|candidate| candidate.id == capability_id)
            .ok_or_else(|| ExtensionError::InvalidManifest {
                reason: format!(
                    "built-in first-party manifest is missing capability {capability_id}"
                ),
            })?;
        descriptor.description.push_str(GUIDANCE);
        manifest.description.push_str(GUIDANCE);
    }
    Ok(())
}

fn process_port_backed_builtins_enabled(process_backend: ProcessBackendKind) -> bool {
    matches!(
        process_backend,
        ProcessBackendKind::Docker
            | ProcessBackendKind::Srt
            | ProcessBackendKind::SmolVm
            | ProcessBackendKind::LocalHost
            | ProcessBackendKind::UserSandbox
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

fn remove_builtin_capability_effect(
    package: &mut ExtensionPackage,
    capability_id: &str,
    effect: EffectKind,
) -> Result<(), ExtensionError> {
    let capability_id = CapabilityId::new(capability_id)?;
    let descriptor = package
        .capabilities
        .iter_mut()
        .find(|candidate| candidate.id == capability_id)
        .ok_or_else(|| ExtensionError::InvalidManifest {
            reason: format!("built-in first-party package is missing capability {capability_id}"),
        })?;
    let manifest = package
        .manifest
        .capabilities
        .iter_mut()
        .find(|candidate| candidate.id == capability_id)
        .ok_or_else(|| ExtensionError::InvalidManifest {
            reason: format!("built-in first-party manifest is missing capability {capability_id}"),
        })?;
    if !descriptor.effects.contains(&effect) || !manifest.effects.contains(&effect) {
        return Err(ExtensionError::InvalidManifest {
            reason: format!(
                "built-in first-party capability {capability_id} is missing effect {effect:?}"
            ),
        });
    }
    descriptor.effects.retain(|candidate| *candidate != effect);
    manifest.effects.retain(|candidate| *candidate != effect);
    Ok(())
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
///
/// `active_run_lookup` is required (not `Option`): the caller-scoped
/// `trigger_list` capability derives its `active_hold` projection from it, so
/// production wiring must always supply the same lookup the automations
/// panel uses (#5886).
pub fn builtin_first_party_handlers_with_trigger_create_hook(
    trigger_repository: Arc<dyn ironclaw_triggers::TriggerRepository>,
    trigger_create_hook: Arc<dyn TriggerCreateHook>,
    active_run_lookup: Arc<dyn ironclaw_triggers::TriggerActiveRunLookup>,
) -> Result<FirstPartyCapabilityRegistry, HostApiError> {
    let mut registry = builtin_first_party_base_registry()?;
    trigger_management::insert_handlers_with_create_hook(
        &mut registry,
        trigger_repository,
        trigger_create_hook,
        active_run_lookup,
    )?;
    Ok(registry)
}

/// Create handlers with the complete trigger service set, including the
/// shared worker-backed manual-fire path used by `builtin.trigger_run`.
pub fn builtin_first_party_handlers_with_trigger_services(
    trigger_repository: Arc<dyn ironclaw_triggers::TriggerRepository>,
    trigger_create_hook: Arc<dyn TriggerCreateHook>,
    active_run_lookup: Arc<dyn ironclaw_triggers::TriggerActiveRunLookup>,
    manual_fire_runner: Arc<dyn ironclaw_triggers::TriggerManualFireRunner>,
) -> Result<FirstPartyCapabilityRegistry, HostApiError> {
    let mut registry = builtin_first_party_base_registry()?;
    trigger_management::insert_handlers_with_services(
        &mut registry,
        trigger_repository,
        trigger_create_hook,
        active_run_lookup,
        manual_fire_runner,
    )?;
    Ok(registry)
}

/// Replace the fail-closed default for the explicit model-delivery capability
/// with the product-owned delivery service selected by composition.
pub fn register_outbound_deliver_first_party_handler(
    registry: &mut FirstPartyCapabilityRegistry,
    delivery: Arc<dyn ironclaw_outbound::ModelChannelDelivery>,
) -> Result<(), HostApiError> {
    outbound_deliver::insert_handler(registry, delivery)
}

/// Replace the fail-closed reply-attachment default with the durable,
/// run-scoped intent store selected by composition.
pub fn register_reply_attachment_first_party_handler(
    registry: &mut FirstPartyCapabilityRegistry,
    intent_port: Arc<dyn ironclaw_outbound::ReplyAttachmentIntentPort>,
) -> Result<(), HostApiError> {
    reply_attachment::insert_handler(registry, intent_port)
}

pub fn builtin_first_party_handlers_with_trigger_create_hook_for_process_backend(
    trigger_repository: Arc<dyn ironclaw_triggers::TriggerRepository>,
    trigger_create_hook: Arc<dyn TriggerCreateHook>,
    active_run_lookup: Arc<dyn ironclaw_triggers::TriggerActiveRunLookup>,
    process_backend: ProcessBackendKind,
) -> Result<FirstPartyCapabilityRegistry, HostApiError> {
    let mut registry = builtin_first_party_handlers_with_trigger_create_hook(
        trigger_repository,
        trigger_create_hook,
        active_run_lookup,
    )?;
    if !process_port_backed_builtins_enabled(process_backend) {
        remove_process_port_backed_builtin_handlers(&mut registry)?;
    }
    Ok(registry)
}

pub fn builtin_first_party_handlers_with_trigger_services_for_process_backend(
    trigger_repository: Arc<dyn ironclaw_triggers::TriggerRepository>,
    trigger_create_hook: Arc<dyn TriggerCreateHook>,
    active_run_lookup: Arc<dyn ironclaw_triggers::TriggerActiveRunLookup>,
    manual_fire_runner: Arc<dyn ironclaw_triggers::TriggerManualFireRunner>,
    process_backend: ProcessBackendKind,
) -> Result<FirstPartyCapabilityRegistry, HostApiError> {
    let mut registry = builtin_first_party_handlers_with_trigger_services(
        trigger_repository,
        trigger_create_hook,
        active_run_lookup,
        manual_fire_runner,
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
        .with_handler(CapabilityId::new(SHELL_CAPABILITY_ID)?, handler.clone());
    insert_coding_handlers(&mut registry)?;
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
        handler,
    );
    outbound_deliver::insert_handler(&mut registry, Arc::new(UnavailableModelChannelDelivery))?;
    reply_attachment::insert_unavailable_handler(&mut registry)?;
    skill_management::insert_handlers(&mut registry)?;
    Ok(registry)
}

struct UnavailableModelChannelDelivery;

#[async_trait]
impl ironclaw_outbound::ModelChannelDelivery for UnavailableModelChannelDelivery {
    async fn deliver_for_model(
        &self,
        _request: ironclaw_outbound::ModelChannelDeliveryRequest,
    ) -> Result<
        ironclaw_outbound::ModelChannelDeliveryEvidence,
        ironclaw_outbound::ModelChannelDeliveryError,
    > {
        Err(ironclaw_outbound::ModelChannelDeliveryError::Unavailable)
    }
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
        description: description.to_string(),
        effects,
        default_permission,
        visibility: CapabilityVisibility::Model,
        standard_op: None,
        input_schema_ref: CapabilityProfileSchemaRef::new(format!(
            "schemas/builtin/{schema_name}.input.v1.json"
        ))?,
        output_schema_ref: Some(CapabilityProfileSchemaRef::new(format!(
            "schemas/builtin/{schema_name}.output.v1.json"
        ))?),
        prompt_doc_ref: None,
        required_host_ports: Vec::new(),
        runtime_credentials: Vec::new(),
        network_targets: Vec::new(),
        max_egress_bytes: None,
        resource_profile,
        origin_gate_matrix: Some(first_party_origin_gate_matrix(id)),
        // The stock builtins never declare a provider-name override; their
        // model-visible names derive from the capability id.
        provider_tool_name: None,
    })
}

fn first_party_origin_gate_matrix(id: &str) -> OriginGateMatrix {
    let mut matrix = OriginGateMatrix::builtin_loop_run_seed(id);
    if matches!(
        id,
        SKILL_INSTALL_CAPABILITY_ID
            | SKILL_UPDATE_CAPABILITY_ID
            | SKILL_AUTO_ACTIVATE_SET_CAPABILITY_ID
            | SKILL_REMOVE_CAPABILITY_ID
    ) {
        matrix.product = OriginGatePolicy::ConsentSufficient;
    }
    matrix
}

#[derive(Debug, Default)]
pub struct BuiltinFirstPartyTools {}

#[async_trait]
impl FirstPartyCapabilityHandler for BuiltinFirstPartyTools {
    async fn dispatch(
        &self,
        mut request: FirstPartyCapabilityRequest,
    ) -> Result<FirstPartyCapabilityResult, FirstPartyCapabilityError> {
        bounded_input_size(&request.input)?;
        normalize_optional_null_sentinels(&mut request);
        let start = Instant::now();
        let mut network_egress_bytes = 0;
        let process_count = 0u32;
        let mut pending_artifact = None;
        let (output, display_preview) = match request.capability_id.as_str() {
            ECHO_CAPABILITY_ID => (echo::dispatch(&request.input)?, None),
            TIME_CAPABILITY_ID => (time::dispatch(&request.input)?, None),
            JSON_CAPABILITY_ID => (json::dispatch(&request).await?, None),
            HTTP_CAPABILITY_ID | HTTP_SAVE_CAPABILITY_ID => {
                let result = http::dispatch(&request).await?;
                network_egress_bytes = result.network_egress_bytes;
                pending_artifact = result.pending_artifact;
                (result.output, None)
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
            // The pinned coding surface (`builtin.read`/`write`/`edit`/`glob`/
            // `grep`) is dispatched by `CodingTools`, registered through
            // `insert_coding_handlers`; this handler owns no coding ids.
            _ => {
                return Err(FirstPartyCapabilityError::new(
                    RuntimeDispatchErrorKind::UndeclaredCapability,
                ));
            }
        };
        let wall_clock_ms = start.elapsed().as_millis().try_into().unwrap_or(u64::MAX);
        let output_limit_bytes = match request.capability_id.as_str() {
            HTTP_CAPABILITY_ID => http::MAX_HTTP_OUTPUT_BYTES,
            HTTP_SAVE_CAPABILITY_ID => FIRST_PARTY_MAX_OUTPUT_BYTES,
            _ => FIRST_PARTY_MAX_OUTPUT_BYTES,
        };
        let output_bytes = bounded_output_bytes(&output, output_limit_bytes).map_err(|error| {
            if network_egress_bytes > 0 || process_count > 0 {
                error.with_usage(
                    ResourceUsage::default()
                        .set_wall_clock_ms(wall_clock_ms)
                        .set_network_egress_bytes(network_egress_bytes)
                        .set_process_count(process_count),
                )
            } else {
                error
            }
        })?;
        let usage = ResourceUsage::default()
            .set_wall_clock_ms(wall_clock_ms)
            .set_output_bytes(output_bytes)
            .set_network_egress_bytes(network_egress_bytes)
            .set_process_count(process_count);
        let result =
            FirstPartyCapabilityResult::new(output, usage).with_display_preview(display_preview);
        Ok(match pending_artifact {
            Some(artifact) => result.with_pending_artifact(artifact),
            None => result,
        })
    }
}

/// Bounded-input check with the default first-party byte cap, shared by every
/// non-coding built-in handler.
pub(super) fn bounded_input_size(
    input: &serde_json::Value,
) -> Result<(), FirstPartyCapabilityError> {
    bounded_input_size_with_max(input, MAX_FIRST_PARTY_INPUT_BYTES)
}

/// Bounded-input check with an explicit byte cap. The coding adapter uses this
/// with its own metadata table (`first_party_tools::coding`).
pub(super) fn bounded_input_size_with_max(
    input: &serde_json::Value,
    max_bytes: usize,
) -> Result<(), FirstPartyCapabilityError> {
    let bytes = serde_json::to_vec(input).map_err(|error| {
        tracing::debug!(%error, "failed to serialize first-party capability input");
        input_error()
    })?;
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
    let bytes = serde_json::to_vec(output).map_err(|error| {
        tracing::debug!(%error, "failed to serialize first-party capability output");
        input_error()
    })?;
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
    let schema =
        resolve_builtin_input_schema_ref(&format!("schemas/builtin/{schema_name}.input.v1.json"));
    normalize_optional_null_sentinels_against_schema(&mut request.input, schema.as_ref());
}

/// Schema-driven core of the null-sentinel normalization, shared with the
/// memory tool handlers (whose schemas come from the bound package's
/// manifest): drop declared-optional fields whose value is `null`/"null".
pub(super) fn normalize_optional_null_sentinels_against_schema(
    input: &mut serde_json::Value,
    schema: Option<&serde_json::Value>,
) {
    let Some(schema) = schema else {
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
    let Some(object) = input.as_object_mut() else {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_builtin_package_exposes_only_coding_and_no_result_reader() {
        let package = builtin_first_party_package().expect("canonical built-in package");
        let provider_names = package
            .capabilities
            .iter()
            .filter_map(|descriptor| {
                descriptor
                    .provider_tool_name
                    .as_ref()
                    .map(|name| name.as_str())
            })
            .collect::<std::collections::BTreeSet<_>>();
        for name in ["read", "write", "edit", "glob", "grep"] {
            assert!(
                provider_names.contains(name),
                "canonical package must expose coding tool {name}"
            );
        }
        let capability_ids = package
            .capabilities
            .iter()
            .map(|descriptor| descriptor.id.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        for retired in [
            "builtin.read_file",
            "builtin.write_file",
            "builtin.list_dir",
            "builtin.apply_patch",
            "builtin.result_read",
        ] {
            assert!(
                !capability_ids.contains(retired),
                "retired capability {retired} must not remain in the canonical package"
            );
        }
    }
}
