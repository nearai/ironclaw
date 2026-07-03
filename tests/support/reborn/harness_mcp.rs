//! Reborn integration-test harness — mock-MCP scaffolding (slice 6).
//!
//! Extracted from `harness.rs` to keep that file focused: this module owns the
//! loopback mock-MCP wiring — the real `McpRuntime` built over a loopback HTTP
//! egress, the test-only `RuntimeHttpEgress` that talks to the in-process
//! `MockMcpServer`, the mock extension package/registry, and the MCP trust +
//! network policies.
//!
//! The single entry point used by the harness is
//! `HostRuntimeCapabilityHarness::mock_mcp_tools` (in `harness.rs`), which calls
//! the `pub(super)` factories here. Everything in this module is test-only and
//! never ships.

#![allow(dead_code)] // Shared by staged Reborn binary-E2E validation ports.

use std::{path::PathBuf, sync::Arc, time::Duration};

use ironclaw_authorization::GrantAuthorizer;
use ironclaw_extensions::{
    CapabilityManifest, CapabilityVisibility, ExtensionManifest, ExtensionPackage,
    ExtensionRegistry, ExtensionRuntime, MANIFEST_SCHEMA_VERSION, ManifestSource,
};
use ironclaw_host_api::{
    CapabilityDescriptor, CapabilityId, CapabilityProfileSchemaRef, EffectKind, ExtensionId,
    NetworkMethod, NetworkPolicy, NetworkScheme, NetworkTargetPattern, PackageId, PermissionMode,
    RequestedTrustClass, RuntimeHttpEgress, RuntimeHttpEgressError, RuntimeHttpEgressRequest,
    RuntimeHttpEgressResponse, RuntimeKind, TrustClass, VirtualPath,
};
use ironclaw_host_runtime::{
    BUILTIN_FIRST_PARTY_PROVIDER, CapabilitySurfaceVersion as HostRuntimeCapabilitySurfaceVersion,
    HostRuntime, HostRuntimeServices, builtin_first_party_handlers,
};
use ironclaw_mcp::{
    McpHostHttpClient, McpHostHttpEgressPlan, McpRuntime, McpRuntimeConfig, McpRuntimeHttpAdapter,
    StaticMcpHostHttpEgressPlanner,
};
use ironclaw_resources::InMemoryResourceGovernor;
use ironclaw_secrets::InMemorySecretStore;
use ironclaw_trust::{AdminConfig, AdminEntry, HostTrustAssignment, HostTrustPolicy};
use serde_json::json;

use super::harness::{LocalDevRootMounts, RecordingRuntimeHttpEgress, local_dev_root_filesystem};

type HarnessResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// Slice 6: concrete loopback MCP runtime — an `McpRuntime` wired over the
/// test-only `LoopbackMcpRuntimeHttpEgress` so the mock-MCP integration test
/// reaches a real in-process HTTP server (no real network).
pub(super) type LoopbackMcpRuntime = McpRuntime<
    McpHostHttpClient<
        McpRuntimeHttpAdapter<Arc<LoopbackMcpRuntimeHttpEgress>>,
        StaticMcpHostHttpEgressPlanner,
    >,
>;

/// Build the concrete loopback MCP runtime used by `mock_mcp_tools`: a real
/// `McpRuntime` whose HTTP egress is the loopback adapter that talks to the
/// test-local mock MCP server.
pub(super) fn build_loopback_mcp_runtime(mcp_url: &str) -> HarnessResult<Arc<LoopbackMcpRuntime>> {
    let mcp_egress = Arc::new(LoopbackMcpRuntimeHttpEgress::new(mcp_url)?);
    let adapter = McpRuntimeHttpAdapter::new(Arc::clone(&mcp_egress));
    let planner = StaticMcpHostHttpEgressPlanner::new(McpHostHttpEgressPlan::default());
    let client = McpHostHttpClient::new(adapter, planner);
    let mcp_runtime: Arc<LoopbackMcpRuntime> =
        Arc::new(McpRuntime::new(McpRuntimeConfig::default(), client));
    Ok(mcp_runtime)
}

/// Slice 6: variant of `local_dev_host_runtime_with_registry_and_runtime_http_egress`
/// that also wires a loopback MCP runtime for the mock-MCP integration test.
///
/// The `first_party_egress` covers any first-party tool calls (recording, no
/// network). The `mcp_runtime` is a concrete loopback runtime that makes real
/// HTTP requests to the test-local mock MCP server.
pub(super) fn local_dev_host_runtime_with_registry_egress_and_mcp(
    storage_root: PathBuf,
    registry: ExtensionRegistry,
    first_party_egress: Arc<RecordingRuntimeHttpEgress>,
    mcp_runtime: Arc<LoopbackMcpRuntime>,
    mcp_provider_id: &str,
) -> HarnessResult<Arc<dyn HostRuntime>> {
    let services = HostRuntimeServices::new(
        Arc::new(registry),
        local_dev_root_filesystem(storage_root, LocalDevRootMounts::core_builtins())?,
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::ProcessServices::in_memory(),
        HostRuntimeCapabilitySurfaceVersion::new("reborn-app-v1")?,
    )
    .with_secret_store(Arc::new(InMemorySecretStore::new()))
    .with_first_party_capabilities(Arc::new(builtin_first_party_handlers(Arc::new(
        ironclaw_triggers::InMemoryTriggerRepository::default(),
    ))?))
    .with_first_party_http_egress(first_party_egress)
    .with_mcp_runtime(mcp_runtime)
    .with_trust_policy(Arc::new(first_party_and_mcp_trust_policy(mcp_provider_id)?));
    Ok(Arc::new(services.host_runtime_for_local_testing()))
}

/// Build an `ExtensionPackage` describing a hosted MCP extension backed by the
/// loopback mock server. `provider_id` is both the extension id and the prefix
/// stripped from capability ids to derive the MCP tool name
/// (`"mock-mcp"` + `"mock-mcp.search"` → MCP tool `"search"`).
/// No `runtime_credentials` are declared because `LoopbackMcpRuntimeHttpEgress`
/// injects the Bearer token directly for test purposes.
///
/// Uses `from_host_bundled_manifest_with_inline_dynamic_schemas` with an inline
/// `{"type":"object"}` parameters_schema so `surface_descriptor` in the host
/// runtime skips the `$ref` filesystem read (no schema file exists for the mock
/// extension). All descriptor fields except `parameters_schema` still match the
/// manifest projection exactly.
pub(super) fn mock_mcp_extension_package(
    provider_id: &str,
    mcp_url: &str,
    capability_id: &str,
) -> HarnessResult<ExtensionPackage> {
    let manifest = ExtensionManifest {
        schema_version: MANIFEST_SCHEMA_VERSION.to_string(),
        id: ExtensionId::new(provider_id)?,
        name: provider_id.to_string(),
        version: "0.1.0".to_string(),
        description: "Mock MCP extension (test only)".to_string(),
        source: ManifestSource::HostBundled,
        requested_trust: RequestedTrustClass::ThirdParty,
        descriptor_trust_default: TrustClass::Sandbox,
        runtime: ExtensionRuntime::Mcp {
            transport: "http".to_string(),
            command: None,
            args: Vec::new(),
            url: Some(mcp_url.to_string()),
        },
        host_apis: Vec::new(),
        hooks: Vec::new(),
        capabilities: vec![CapabilityManifest {
            id: CapabilityId::new(capability_id)?,
            implements: Vec::new(),
            description: "Mock MCP capability".to_string(),
            effects: vec![EffectKind::DispatchCapability, EffectKind::Network],
            default_permission: PermissionMode::Allow,
            visibility: CapabilityVisibility::Model,
            input_schema_ref: CapabilityProfileSchemaRef::new(
                "schemas/mock-mcp/mock.input.v1.json",
            )?,
            output_schema_ref: CapabilityProfileSchemaRef::new(
                "schemas/mock-mcp/mock.output.v1.json",
            )?,
            prompt_doc_ref: None,
            required_host_ports: Vec::new(),
            runtime_credentials: Vec::new(),
            resource_profile: None,
        }],
    };
    // Inline schema so surface_descriptor returns Ok(descriptor) without
    // trying to read "schemas/mock-mcp/mock.input.v1.json" from the test
    // filesystem (that file doesn't exist for a test-only mock extension).
    let capabilities = vec![CapabilityDescriptor {
        id: CapabilityId::new(capability_id)?,
        provider: ExtensionId::new(provider_id)?,
        runtime: RuntimeKind::Mcp,
        trust_ceiling: TrustClass::Sandbox,
        description: "Mock MCP capability".to_string(),
        parameters_schema: json!({"type": "object"}),
        effects: vec![EffectKind::DispatchCapability, EffectKind::Network],
        default_permission: PermissionMode::Allow,
        runtime_credentials: Vec::new(),
        resource_profile: None,
    }];
    let root = VirtualPath::new(format!("/system/extensions/{provider_id}"))?;
    Ok(
        ExtensionPackage::from_host_bundled_manifest_with_inline_dynamic_schemas(
            manifest,
            root,
            None,
            capabilities,
        )?,
    )
}

/// Trust policy for MCP integration tests: first-party builtins + user-trusted
/// mock MCP provider.  The mock MCP provider is registered with root
/// `/system/extensions/<provider_id>`, so its manifest path must match the
/// `PackageSource::LocalManifest` key the host runtime derives at dispatch time.
fn first_party_and_mcp_trust_policy(mcp_provider_id: &str) -> HarnessResult<HostTrustPolicy> {
    Ok(HostTrustPolicy::new(vec![Box::new(
        AdminConfig::with_entries(vec![
            AdminEntry::for_local_manifest(
                PackageId::new(BUILTIN_FIRST_PARTY_PROVIDER)?,
                "/system/extensions/builtin/manifest.toml".to_string(),
                None,
                HostTrustAssignment::first_party(),
                vec![
                    EffectKind::DispatchCapability,
                    EffectKind::ReadFilesystem,
                    EffectKind::WriteFilesystem,
                    EffectKind::DeleteFilesystem,
                    EffectKind::Network,
                    EffectKind::SpawnProcess,
                    EffectKind::ExecuteCode,
                    EffectKind::ExternalWrite,
                ],
                None,
            ),
            AdminEntry::for_local_manifest(
                PackageId::new(mcp_provider_id)?,
                format!("/system/extensions/{mcp_provider_id}/manifest.toml"),
                None,
                HostTrustAssignment::user_trusted(),
                vec![EffectKind::DispatchCapability, EffectKind::Network],
                None,
            ),
        ]),
    )])?)
}

/// Network policy for the slice-6 loopback mock MCP server. The mock binds to
/// `http://127.0.0.1:<port>/mcp`, so the policy must permit the loopback host and
/// must NOT deny private/loopback IP ranges (127.0.0.1 is loopback). An empty
/// `allowed_targets` (the `NetworkPolicy::default()`) is rejected by the host
/// runtime's network obligation, which is what previously blocked the MCP egress.
pub(super) fn mcp_loopback_network_policy() -> NetworkPolicy {
    NetworkPolicy {
        allowed_targets: vec![NetworkTargetPattern {
            scheme: Some(NetworkScheme::Http),
            host_pattern: "127.0.0.1".to_string(),
            port: None,
        }],
        deny_private_ip_ranges: false,
        max_egress_bytes: Some(1_000_000),
    }
}

/// Test-only `RuntimeHttpEgress` that routes MCP traffic to the loopback mock
/// MCP server using a real HTTP client (slice 6 design).
///
/// Unlike `RecordingRuntimeHttpEgress`, this makes REAL HTTP connections so the
/// `MockMcpServer` actually receives the JSON-RPC handshake. It:
///   - rejects any URL that does not start with the configured mock endpoint
///     (hermetic guard — prevents accidental real-network egress)
///   - injects `Authorization: Bearer mock-mcp-test-token` on every request,
///     satisfying the mock server's OAuth gate without a credential-staging
///     pipeline (acceptable because this egress is test-only and never ships)
///   - passes all other request headers through unchanged
pub(super) struct LoopbackMcpRuntimeHttpEgress {
    /// Full MCP endpoint URL (e.g. `"http://127.0.0.1:PORT/mcp"`).
    /// All outbound URLs must start with this value — hermetic guard.
    mcp_url: String,
    client: reqwest::Client,
}

impl LoopbackMcpRuntimeHttpEgress {
    fn new(mcp_url: &str) -> HarnessResult<Self> {
        // Hermetic hardening: refuse any host other than 127.0.0.1 so a typo in
        // the mock URL cannot silently turn this test egress into real external
        // network I/O. Narrowed to 127.0.0.1 only (not ::1 / localhost) so the
        // guard matches `mcp_loopback_network_policy()`, which also only permits
        // 127.0.0.1; a caller using "localhost" would otherwise pass this guard
        // then fail network authorization — a latent trap.
        let parsed = url::Url::parse(mcp_url)
            .map_err(|e| format!("invalid mock MCP URL {mcp_url:?}: {e}"))?;
        let scheme = parsed.scheme();
        if scheme != "http" {
            return Err(format!(
                "mock MCP URL {mcp_url:?} must use http://127.0.0.1/...; scheme {scheme:?} not \
                 accepted (mcp_loopback_network_policy only permits http)"
            )
            .into());
        }
        let is_loopback_ipv4 = match parsed.host() {
            Some(url::Host::Ipv4(ip)) => ip == std::net::Ipv4Addr::LOCALHOST,
            _ => false,
        };
        if !is_loopback_ipv4 {
            return Err(format!(
                "mock MCP URL {mcp_url:?} host is not 127.0.0.1; only the IPv4 loopback \
                 address is accepted (matches mcp_loopback_network_policy); refusing \
                 non-hermetic egress"
            )
            .into());
        }
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            // Disable automatic redirect-following so a mock 3xx cannot redirect
            // the client off loopback. The start_with(mcp_url) hermetic guard only
            // checks the first request URL; a followed redirect to an external host
            // would bypass it entirely.
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| format!("failed to build reqwest client for mock MCP egress: {e}"))?;
        Ok(Self {
            mcp_url: mcp_url.to_string(),
            client,
        })
    }
}

#[async_trait::async_trait]
impl RuntimeHttpEgress for LoopbackMcpRuntimeHttpEgress {
    async fn execute(
        &self,
        request: RuntimeHttpEgressRequest,
    ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
        // Hermetic guard: only route to the configured loopback mock endpoint.
        if !request.url.starts_with(&self.mcp_url) {
            return Err(RuntimeHttpEgressError::Request {
                reason: format!(
                    "loopback MCP egress: URL {:?} is outside allowed mock endpoint {:?}",
                    request.url, self.mcp_url,
                ),
                request_bytes: 0,
                response_bytes: 0,
            });
        }
        let request_bytes = request.body.len() as u64;
        let method = match request.method {
            NetworkMethod::Get => reqwest::Method::GET,
            NetworkMethod::Post => reqwest::Method::POST,
            NetworkMethod::Put => reqwest::Method::PUT,
            NetworkMethod::Patch => reqwest::Method::PATCH,
            NetworkMethod::Delete => reqwest::Method::DELETE,
            NetworkMethod::Head => reqwest::Method::HEAD,
        };
        let mut builder = self.client.request(method, &request.url);
        for (name, value) in &request.headers {
            builder = builder.header(name.as_str(), value.as_str());
        }
        // The mock server requires a non-empty Bearer token on every request.
        // Inject a fixed test token since there is no credential-staging
        // pipeline in this test-only egress path.
        builder = builder.header("authorization", "Bearer mock-mcp-test-token");
        if !request.body.is_empty() {
            builder = builder.body(request.body.clone());
        }
        let response = builder
            .send()
            .await
            .map_err(|e| RuntimeHttpEgressError::Network {
                reason: e.to_string(),
                request_bytes,
                response_bytes: 0,
            })?;
        let status = response.status().as_u16();
        let headers: Vec<(String, String)> = response
            .headers()
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
            .collect();
        let body = response
            .bytes()
            .await
            .map_err(|e| RuntimeHttpEgressError::Network {
                reason: e.to_string(),
                request_bytes,
                response_bytes: 0,
            })?;
        let response_bytes = body.len() as u64;
        Ok(RuntimeHttpEgressResponse {
            status,
            headers,
            body: body.to_vec(),
            saved_body: None,
            request_bytes,
            response_bytes,
            redaction_applied: false,
        })
    }
}
