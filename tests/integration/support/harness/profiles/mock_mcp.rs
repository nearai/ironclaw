//! Mock-MCP domain tools profiles.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use super::super::super::harness_mcp::{
    MockMcpPackage, build_loopback_mcp_runtime, mcp_loopback_network_policy,
    mock_mcp_extension_package, mock_mcp_multi_tool_extension_package,
    standalone_host_runtime_with_registry_egress_and_mcp,
};
use super::super::{
    HarnessResult, HostRuntimeCapabilityHarness, RecordingRuntimeHttpEgress,
    host_runtime_storage_roots, workspace_mounts,
};
use ironclaw_extension_registry::ExtensionRegistry;
use ironclaw_host_api::{
    capability::EffectKind,
    ids::{CapabilityId, ExtensionId, UserId},
    mount::MountPermissions,
    runtime::RuntimeKind,
};

/// Wire a single MCP capability backed by the loopback mock server.
///
/// `mcp_url`  — the mock server's MCP endpoint (e.g. `"http://127.0.0.1:PORT/mcp"`).
/// `provider_id`   — extension id used in the registry (e.g. `"mock-mcp"`).
/// `capability_id` — capability id surfaced to the model (e.g. `"mock-mcp.search"`).
///
/// The harness (via the `harness_mcp` scaffolding) builds a loopback MCP
/// egress that makes REAL HTTP connections to the mock server, injecting a
/// fake Bearer token to satisfy the mock's auth gate. Production egress
/// policy, network policy, and credential stores are bypassed — this path is
/// test-only.
pub(crate) async fn mock_mcp_tools(
    mcp_url: &str,
    provider_id: &str,
    capability_id: &str,
) -> HarnessResult<HostRuntimeCapabilityHarness> {
    let (root, storage_root, workspace_root) = host_runtime_storage_roots()?;
    // Recording egress for any first-party tool paths (unused in MCP tests,
    // but HostRuntimeServices requires it when first_party_capabilities are wired).
    let first_party_egress = Arc::new(RecordingRuntimeHttpEgress::with_body(
        br#"{"accepted":true}"#.to_vec(),
    ));
    // Real loopback egress + MCP runtime for the mock MCP server; the
    // scaffolding (egress, adapter chain, runtime) lives in `harness_mcp`.
    let mcp_runtime = build_loopback_mcp_runtime(mcp_url)?;
    let mut registry = ExtensionRegistry::new();
    registry.insert(mock_mcp_extension_package(
        provider_id,
        mcp_url,
        capability_id,
    )?)?;
    let runtime = standalone_host_runtime_with_registry_egress_and_mcp(
        storage_root,
        registry,
        Arc::clone(&first_party_egress),
        mcp_runtime,
        &[provider_id],
    )?;
    mock_mcp_harness(
        runtime,
        root,
        workspace_root,
        vec![CapabilityId::new(capability_id)?],
        ExtensionId::new(provider_id)?,
        Vec::new(),
    )
}

/// Wire several mock MCP packages, all served by the one loopback mock
/// server at `mcp_url`, plus the synthetic `result_read` capability so a
/// large MCP result can be paged the way production pages it. The first
/// package is the harness's primary provider; the others are trusted
/// alongside it.
pub(crate) async fn mock_mcp_package_tools(
    mcp_url: &str,
    packages: &[MockMcpPackage],
) -> HarnessResult<HostRuntimeCapabilityHarness> {
    let Some((primary, others)) = packages.split_first() else {
        return Err("mock_mcp_package_tools needs at least one package".into());
    };
    let (root, storage_root, workspace_root) = host_runtime_storage_roots()?;
    let first_party_egress = Arc::new(RecordingRuntimeHttpEgress::with_body(
        br#"{"accepted":true}"#.to_vec(),
    ));
    let mcp_runtime = build_loopback_mcp_runtime(mcp_url)?;
    let mut registry = ExtensionRegistry::new();
    let mut capability_ids = Vec::new();
    for package in packages {
        registry.insert(mock_mcp_multi_tool_extension_package(package, mcp_url)?)?;
        for tool in &package.tools {
            capability_ids.push(CapabilityId::new(tool.capability_id.as_str())?);
        }
    }
    capability_ids.push(CapabilityId::new(
        ironclaw_composition::test_support::RESULT_READ_CAPABILITY_ID,
    )?);
    let provider_ids: Vec<&str> = packages
        .iter()
        .map(|package| package.provider_id.as_str())
        .collect();
    let runtime = standalone_host_runtime_with_registry_egress_and_mcp(
        storage_root,
        registry,
        Arc::clone(&first_party_egress),
        mcp_runtime,
        &provider_ids,
    )?;
    let additional_provider_trust = others
        .iter()
        .map(|package| {
            Ok((
                ExtensionId::new(package.provider_id.as_str())?,
                vec![EffectKind::DispatchCapability, EffectKind::Network],
            ))
        })
        .collect::<HarnessResult<Vec<_>>>()?;
    mock_mcp_harness(
        runtime,
        root,
        workspace_root,
        capability_ids,
        ExtensionId::new(primary.provider_id.as_str())?,
        additional_provider_trust,
    )
}

fn mock_mcp_harness(
    runtime: Arc<dyn ironclaw_host_runtime::HostRuntime>,
    root: Arc<tempfile::TempDir>,
    workspace_root: std::path::PathBuf,
    capability_ids: Vec<CapabilityId>,
    provider_id: ExtensionId,
    additional_provider_trust: Vec<(ExtensionId, Vec<EffectKind>)>,
) -> HarnessResult<HostRuntimeCapabilityHarness> {
    let mounts = workspace_mounts(MountPermissions::read_write_list_delete())?;
    let (io, result_writer_io) = super::super::default_capability_io_pair();
    Ok(HostRuntimeCapabilityHarness {
        runtime: Mutex::new(runtime),
        resource_governor: None,
        approval_parts: None,
        gate_record_store: super::super::fresh_in_memory_gate_record_store(),
        auto_approve_settings: None,
        pending_approval_scopes: Arc::new(Mutex::new(HashMap::new())),
        io: Mutex::new(io),
        result_writer_io: Mutex::new(result_writer_io),
        durable_capability_io_thread_service: Mutex::new(None),
        // Production composes `StagedCapabilityIo`; keep this lane on that
        // canonical durable writer so MCP result references exercise the same
        // result_read contract as deployed calls.
        durable_capability_io_requested: true,
        root,
        workspace_root,
        mounts,
        capability_mount_overrides: Vec::new(),
        capability_ids,
        runtime_kind: RuntimeKind::Mcp,
        effect_kinds: vec![EffectKind::DispatchCapability, EffectKind::Network],
        // The MCP capability declares `EffectKind::Network`, so authorization
        // attaches an `ApplyNetworkPolicy` obligation that the host runtime
        // rejects when `allowed_targets` is empty (a default `NetworkPolicy`).
        // The mock server lives at `http://127.0.0.1:<port>/mcp`, so permit the
        // loopback host (and disable the private-IP denial that would otherwise
        // block 127.0.0.1) so the MCP egress reaches the loopback server.
        network_policy: mcp_loopback_network_policy(),
        secrets: Vec::new(),
        provider_id,
        additional_provider_trust,
        user_id: UserId::new("reborn-itest-mcp-user")?,
        invocations: Arc::new(Mutex::new(Vec::new())),
        results: Arc::new(Mutex::new(Vec::new())),
        http_egress: None,
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
        trigger_active_run_lookup_requested: false,
    })
}
