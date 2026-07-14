//! web_access domain capability profile.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use super::super::super::harness_web_access;
use super::super::{
    HarnessResult, HostRuntimeCapabilityHarness, RecordingRuntimeHttpEgress,
    host_runtime_storage_roots, workspace_mounts,
};
use ironclaw_extensions::ExtensionRegistry;
use ironclaw_first_party_extensions::{WEB_GET_CONTENT_CAPABILITY_ID, WEB_SEARCH_CAPABILITY_ID};
use ironclaw_host_api::{
    CapabilityId, EffectKind, ExtensionId, MountPermissions, RuntimeKind, UserId,
};

/// Real capability ids `web_access_tools` registers on the built harness —
/// a single source shared with the wiring-parity capability-id subset check
/// (`tests/integration/wiring_parity.rs`) instead of a second
/// hand-transcribed copy of this list.
pub(crate) fn web_access_tools_capability_ids() -> HarnessResult<Vec<CapabilityId>> {
    Ok(vec![
        CapabilityId::new(WEB_SEARCH_CAPABILITY_ID)?,
        CapabilityId::new(WEB_GET_CONTENT_CAPABILITY_ID)?,
    ])
}

/// C-WEBACCESS: wires the real first-party web-access capabilities through production's
/// `WebAccessExecutor`; no credential-injecting authorizer needed (declares zero `runtime_credentials`).
///
/// Exa MCP's three-leg handshake shares one URL, so responses are scripted via
/// `RecordingRuntimeHttpEgress::push_response_body` (FIFO), not the keyed matcher.
pub(crate) async fn web_access_tools() -> HarnessResult<HostRuntimeCapabilityHarness> {
    let (root, storage_root, workspace_root) = host_runtime_storage_roots()?;
    let http_egress = Arc::new(RecordingRuntimeHttpEgress::with_body(
        br#"{"accepted":true}"#.to_vec(),
    ));
    let mut registry = ExtensionRegistry::new();
    registry.insert(harness_web_access::web_access_extension_package()?)?;
    let runtime = harness_web_access::local_dev_host_runtime_with_web_access(
        storage_root,
        registry,
        Arc::clone(&http_egress),
    )?;
    let mounts = workspace_mounts(MountPermissions::read_write_list_delete())?;
    let (io, result_writer_io) = super::super::default_capability_io_pair();
    Ok(HostRuntimeCapabilityHarness {
        runtime,
        approval_parts: None,
        auto_approve_settings: None,
        pending_approval_scopes: Arc::new(Mutex::new(HashMap::new())),
        io: Mutex::new(io),
        result_writer_io: Mutex::new(result_writer_io),
        durable_capability_io_thread_service: Mutex::new(None),
        durable_capability_io_requested: false,
        root,
        workspace_root,
        mounts,
        capability_mount_overrides: Vec::new(),
        capability_ids: web_access_tools_capability_ids()?,
        runtime_kind: RuntimeKind::FirstParty,
        effect_kinds: vec![EffectKind::DispatchCapability, EffectKind::Network],
        network_policy: harness_web_access::exa_mcp_test_network_policy(),
        secrets: Vec::new(),
        provider_id: ExtensionId::new(harness_web_access::WEB_ACCESS_PROVIDER_ID)?,
        additional_provider_trust: Vec::new(),
        user_id: UserId::new("reborn-itest-web-access-user")?,
        invocations: Arc::new(Mutex::new(Vec::new())),
        results: Arc::new(Mutex::new(Vec::new())),
        http_egress: Some(http_egress),
        network_egress: None,
        real_egress_transport: None,
        process_port: None,
        profile_filesystem: None,
        project_service: None,
        skill_activation_source: None,
        attachment_test_support: None,
        inbound_attachment_reader: None,
        outbound_target_tools: None,
        scope_capability_by_run_owner: false,
        product_auth: None,
        tool_permission_overrides: None,
        persistent_approval_policies: None,
        trigger_repository: None,
        reborn_services: None,
    })
}
