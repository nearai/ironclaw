//! web_access domain tools profiles (populated by the profile migration).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use ironclaw_extensions::ExtensionRegistry;
use ironclaw_first_party_extensions::{WEB_GET_CONTENT_CAPABILITY_ID, WEB_SEARCH_CAPABILITY_ID};
use ironclaw_host_api::{
    CapabilityId, EffectKind, ExtensionId, MountPermissions, RuntimeKind, UserId,
};
use ironclaw_reborn_composition::ProductLiveCapabilityIo;

use super::super::super::harness_web_access;
use super::super::{
    HarnessResult, HostRuntimeCapabilityHarness, RecordingRuntimeHttpEgress,
    host_runtime_storage_roots, workspace_mounts,
};

/// C-WEBACCESS: wires the real first-party `web-access.search` /
/// `web-access.get_content` capabilities via the production
/// `register_bundled_web_access_first_party_handlers` registration
/// (`harness_web_access.rs`), which dispatches through the same
/// `WebAccessExecutor` production composition uses. Unlike
/// `github_issue_tools`, no credential-injecting authorizer is needed —
/// web-access declares zero `runtime_credentials` — so this wires the
/// plain default `GrantAuthorizer`.
///
/// The three-leg Exa MCP handshake (`initialize` → `notifications/initialized`
/// → `tools/call`) all target the same URL, so script it via
/// `RecordingRuntimeHttpEgress::push_response_body` (FIFO), not the keyed
/// matcher — see [`install_web_access_responses`](Self::install_web_access_responses),
/// called from `RebornIntegrationHarnessBuilder::build` before the harness
/// is returned.
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
    Ok(HostRuntimeCapabilityHarness {
        runtime,
        approval_parts: None,
        auto_approve_settings: None,
        pending_approval_scopes: Arc::new(Mutex::new(HashMap::new())),
        io: Arc::new(ProductLiveCapabilityIo::default()),
        root,
        workspace_root,
        mounts,
        capability_mount_overrides: Vec::new(),
        capability_ids: vec![
            CapabilityId::new(WEB_SEARCH_CAPABILITY_ID)?,
            CapabilityId::new(WEB_GET_CONTENT_CAPABILITY_ID)?,
        ],
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
        process_port: None,
        profile_filesystem: None,
        project_service: None,
        skill_activation_source: None,
        attachment_test_support: None,
        outbound_target_tools: None,
        scope_capability_by_run_owner: false,
        product_auth: None,
        tool_permission_overrides: None,
    })
}
