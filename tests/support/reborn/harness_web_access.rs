//! Reborn integration-test harness — first-party `web-access.*` scaffolding
//! (C-WEBACCESS).
//!
//! `web-access.search` / `web-access.get_content` are `RuntimeKind::FirstParty`
//! capabilities (`crates/ironclaw_first_party_extensions/assets/web-access/manifest.toml`),
//! not MCP-extension capabilities — this module does NOT reuse `harness_mcp.rs`'s
//! `McpRuntime` scaffolding. The real dispatch logic is
//! `ironclaw_first_party_extensions::web_access::WebAccessExecutor::dispatch`,
//! which itself speaks MCP JSON-RPC by hand over three sequential
//! `RuntimeHttpEgress` calls (`initialize` → `notifications/initialized` →
//! `tools/call`) to the Exa MCP endpoint. `WebAccessTestHandler` below is a
//! thin adapter mirroring production's `WebAccessFirstPartyHandler`
//! (`crates/ironclaw_reborn_composition/src/web_access.rs`, `pub(crate)` to
//! that crate and therefore unreachable from here) — only the adapter glue is
//! re-authored; the dispatch logic executed is the real `WebAccessExecutor`.
//!
//! web-access declares zero `runtime_credentials` and never sets
//! `credential_injections`, so no credential-injecting authorizer is needed —
//! `HostRuntimeCapabilityHarness::web_access_tools` (in `harness.rs`) wires the
//! plain default `GrantAuthorizer`, the same authorizer `core_builtin_tools()`
//! uses for `builtin.http` (also a `Network`-effect capability).

#![allow(dead_code)] // Test-only scaffolding; not every consumer exercises every helper.

use std::{path::PathBuf, sync::Arc};

use async_trait::async_trait;
use ironclaw_authorization::GrantAuthorizer;
use ironclaw_extensions::{
    CapabilityManifest, CapabilityVisibility, ExtensionManifest, ExtensionPackage,
    ExtensionRegistry, ExtensionRuntime, MANIFEST_SCHEMA_VERSION, ManifestSource,
};
use ironclaw_first_party_extensions::{
    EXA_MCP_HOST, NETWORK_EGRESS_LIMIT, WEB_GET_CONTENT_CAPABILITY_ID, WEB_SEARCH_CAPABILITY_ID,
    WebAccessDispatchError, WebAccessDispatchRequest, WebAccessExecutor,
};
use ironclaw_host_api::{
    CapabilityDescriptor, CapabilityId, CapabilityProfileSchemaRef, EffectKind, ExtensionId,
    NetworkPolicy, NetworkScheme, NetworkTargetPattern, PackageId, PermissionMode,
    RequestedTrustClass, RuntimeKind, TrustClass, VirtualPath,
};
use ironclaw_host_runtime::{
    CapabilitySurfaceVersion as HostRuntimeCapabilitySurfaceVersion, FirstPartyCapabilityError,
    FirstPartyCapabilityHandler, FirstPartyCapabilityRegistry, FirstPartyCapabilityRequest,
    FirstPartyCapabilityResult, HostRuntime, HostRuntimeServices,
};
use ironclaw_resources::InMemoryResourceGovernor;
use ironclaw_secrets::InMemorySecretStore;
use ironclaw_trust::{AdminConfig, AdminEntry, HostTrustAssignment, HostTrustPolicy};
use serde_json::json;

use super::harness::{LocalDevRootMounts, RecordingRuntimeHttpEgress, local_dev_root_filesystem};

type HarnessResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// Extension id (== manifest `service`) for the test-only web-access package.
pub(super) const WEB_ACCESS_PROVIDER_ID: &str = "web-access";

/// Thin `FirstPartyCapabilityHandler` adapter mirroring production's
/// `WebAccessFirstPartyHandler` 1:1 — only the adapter glue is re-authored,
/// the dispatch logic is the real, fully-`pub` `WebAccessExecutor::dispatch`.
struct WebAccessTestHandler {
    executor: WebAccessExecutor,
}

#[async_trait]
impl FirstPartyCapabilityHandler for WebAccessTestHandler {
    async fn dispatch(
        &self,
        request: FirstPartyCapabilityRequest,
    ) -> Result<FirstPartyCapabilityResult, FirstPartyCapabilityError> {
        let result = self
            .executor
            .dispatch(WebAccessDispatchRequest {
                capability_id: &request.capability_id,
                scope: &request.scope,
                input: &request.input,
                runtime_http_egress: request.services.runtime_http_egress.clone(),
            })
            .await
            .map_err(web_access_test_error)?;
        Ok(FirstPartyCapabilityResult::new(result.output, result.usage))
    }
}

fn web_access_test_error(error: WebAccessDispatchError) -> FirstPartyCapabilityError {
    let mapped = FirstPartyCapabilityError::new(error.kind());
    match error.usage() {
        Some(usage) => mapped.with_usage(usage.clone()),
        None => mapped,
    }
}

/// Build the `ExtensionPackage` for the test-only `web-access` provider, via
/// `from_host_bundled_manifest_with_inline_dynamic_schemas` (the same helper
/// `mock_mcp_extension_package` in `harness_mcp.rs` uses) with
/// `runtime: ExtensionRuntime::FirstParty { service: "web-access" }` and the
/// two capability manifests copied from
/// `crates/ironclaw_first_party_extensions/assets/web-access/manifest.toml`.
/// Inline `{"type":"object"}` schemas avoid a `$ref` filesystem read for
/// schema files this test harness does not mount.
pub(super) fn web_access_extension_package() -> HarnessResult<ExtensionPackage> {
    let (search_manifest, search_descriptor) = web_access_capability_pair(
        WEB_SEARCH_CAPABILITY_ID,
        "Search the web with zero-config Exa MCP and return cited source results.",
    )?;
    let (get_content_manifest, get_content_descriptor) = web_access_capability_pair(
        WEB_GET_CONTENT_CAPABILITY_ID,
        "Retrieve full web page content through Exa MCP, or read content cached from a \
         previous web search response.",
    )?;
    let manifest = ExtensionManifest {
        schema_version: MANIFEST_SCHEMA_VERSION.to_string(),
        id: ExtensionId::new(WEB_ACCESS_PROVIDER_ID)?,
        name: "Web Access".to_string(),
        version: "0.1.0".to_string(),
        description: "Zero-config web search through Exa MCP for Reborn (test only).".to_string(),
        source: ManifestSource::HostBundled,
        requested_trust: RequestedTrustClass::FirstPartyRequested,
        // Effective first-party trust is assigned by host policy at
        // invocation/surface time (`web_access_first_party_trust_policy`);
        // descriptor trust stays conservative, matching `builtin_first_party_package()`.
        descriptor_trust_default: TrustClass::Sandbox,
        runtime: ExtensionRuntime::FirstParty {
            service: WEB_ACCESS_PROVIDER_ID.to_string(),
        },
        host_apis: Vec::new(),
        hooks: Vec::new(),
        capabilities: vec![search_manifest, get_content_manifest],
    };
    let root = VirtualPath::new(format!("/system/extensions/{WEB_ACCESS_PROVIDER_ID}"))?;
    Ok(
        ExtensionPackage::from_host_bundled_manifest_with_inline_dynamic_schemas(
            manifest,
            root,
            None,
            vec![search_descriptor, get_content_descriptor],
        )?,
    )
}

/// Build a manifest/descriptor pair for one web-access capability. Built
/// together from shared `description` so the two projections cannot drift —
/// `from_host_bundled_manifest_with_inline_dynamic_schemas` requires every
/// descriptor field except `parameters_schema` to match the manifest's own
/// projection exactly.
fn web_access_capability_pair(
    capability_id: &str,
    description: &str,
) -> HarnessResult<(CapabilityManifest, CapabilityDescriptor)> {
    let short_name = capability_id
        .strip_prefix("web-access.")
        .unwrap_or(capability_id);
    let manifest = CapabilityManifest {
        id: CapabilityId::new(capability_id)?,
        implements: Vec::new(),
        description: description.to_string(),
        effects: vec![EffectKind::DispatchCapability, EffectKind::Network],
        default_permission: PermissionMode::Allow,
        visibility: CapabilityVisibility::Model,
        input_schema_ref: CapabilityProfileSchemaRef::new(format!(
            "schemas/web-access/{short_name}.input.v1.json"
        ))?,
        output_schema_ref: CapabilityProfileSchemaRef::new(format!(
            "schemas/web-access/{short_name}.output.v1.json"
        ))?,
        prompt_doc_ref: None,
        required_host_ports: Vec::new(),
        runtime_credentials: Vec::new(),
        resource_profile: None,
    };
    let descriptor = CapabilityDescriptor {
        id: CapabilityId::new(capability_id)?,
        provider: ExtensionId::new(WEB_ACCESS_PROVIDER_ID)?,
        runtime: RuntimeKind::FirstParty,
        trust_ceiling: TrustClass::Sandbox,
        description: description.to_string(),
        parameters_schema: json!({"type": "object"}),
        effects: vec![EffectKind::DispatchCapability, EffectKind::Network],
        default_permission: PermissionMode::Allow,
        runtime_credentials: Vec::new(),
        resource_profile: None,
    };
    Ok((manifest, descriptor))
}

/// Trust policy admitting the test-only `web-access` provider as first-party,
/// mirroring `first_party_trust_policy()`/`github_first_party_trust_policy()`
/// in `harness.rs`. The manifest path must match the `PackageSource::LocalManifest`
/// key the host runtime derives from `web_access_extension_package()`'s root.
pub(super) fn web_access_first_party_trust_policy() -> HarnessResult<HostTrustPolicy> {
    Ok(HostTrustPolicy::new(vec![Box::new(
        AdminConfig::with_entries(vec![AdminEntry::for_local_manifest(
            PackageId::new(WEB_ACCESS_PROVIDER_ID)?,
            format!("/system/extensions/{WEB_ACCESS_PROVIDER_ID}/manifest.toml"),
            None,
            HostTrustAssignment::first_party(),
            vec![EffectKind::DispatchCapability, EffectKind::Network],
            None,
        )]),
    )])?)
}

/// Network policy restricted to the Exa MCP host, mirroring production's
/// private `exa_mcp_network_policy()`
/// (`crates/ironclaw_first_party_extensions/src/web_access.rs`, not `pub`) —
/// re-declared here rather than imported.
pub(super) fn exa_mcp_test_network_policy() -> NetworkPolicy {
    NetworkPolicy {
        allowed_targets: vec![NetworkTargetPattern {
            scheme: Some(NetworkScheme::Https),
            host_pattern: EXA_MCP_HOST.to_string(),
            port: None,
        }],
        deny_private_ip_ranges: true,
        max_egress_bytes: Some(NETWORK_EGRESS_LIMIT),
    }
}

/// Variant of `local_dev_host_runtime_with_registry_and_runtime_http_egress`
/// (`harness.rs`) that wires the `web-access` package registry plus a
/// `FirstPartyCapabilityRegistry` carrying `WebAccessTestHandler` for both
/// capability ids, instead of the built-in first-party handler set.
pub(super) fn local_dev_host_runtime_with_web_access(
    storage_root: PathBuf,
    package_registry: ExtensionRegistry,
    http_egress: Arc<RecordingRuntimeHttpEgress>,
) -> HarnessResult<Arc<dyn HostRuntime>> {
    let handler = Arc::new(WebAccessTestHandler {
        executor: WebAccessExecutor::default(),
    });
    let mut handlers = FirstPartyCapabilityRegistry::new();
    handlers.insert_handler(
        CapabilityId::new(WEB_SEARCH_CAPABILITY_ID)?,
        Arc::clone(&handler),
    );
    handlers.insert_handler(CapabilityId::new(WEB_GET_CONTENT_CAPABILITY_ID)?, handler);

    let services = HostRuntimeServices::new(
        Arc::new(package_registry),
        local_dev_root_filesystem(storage_root, LocalDevRootMounts::core_builtins())?,
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::ProcessServices::in_memory(),
        HostRuntimeCapabilitySurfaceVersion::new("reborn-app-v1")?,
    )
    .with_secret_store(Arc::new(InMemorySecretStore::new()))
    .with_first_party_capabilities(Arc::new(handlers))
    .with_first_party_http_egress(http_egress)
    .with_trust_policy(Arc::new(web_access_first_party_trust_policy()?));

    Ok(Arc::new(services.host_runtime_for_local_testing()))
}
