//! Extension tool binder pins (TOOL-6's lane leg, LIFE-3's missing-lane
//! error): a package prebinds to its configured lane as one behavior-only
//! [`ToolAdapter`] per extension, routing internally by capability id, with
//! the auth gate payload preserved across the ABI.

use std::collections::BTreeSet;

use ironclaw_host_api::{
    RequestedTrustClass, ToolCall, ToolCallResources, ToolError, ToolPorts, TrustClass, VirtualPath,
};

use super::super::ExtensionToolBindError;
use super::*;

fn first_party_test_package(service: &str, capability_id: &str) -> ExtensionPackage {
    ExtensionPackage::from_manifest(
        ExtensionManifest {
            schema_version: ironclaw_extensions::MANIFEST_SCHEMA_VERSION.to_string(),
            id: ExtensionId::new(service).unwrap(),
            name: "Binder fixture".to_string(),
            version: "0.1.0".to_string(),
            description: "extension tool binder fixture".to_string(),
            source: ManifestSource::HostBundled,
            requested_trust: RequestedTrustClass::FirstPartyRequested,
            descriptor_trust_default: TrustClass::Sandbox,
            runtime: ironclaw_extensions::ExtensionRuntime::FirstParty {
                service: service.to_string(),
            },
            host_apis: Vec::new(),
            host_api_surfaces: Vec::new(),
            capabilities: vec![ironclaw_extensions::CapabilityManifest {
                id: CapabilityId::new(capability_id).unwrap(),
                description: "binder fixture capability".to_string(),
                effects: vec![EffectKind::DispatchCapability],
                network_targets: Vec::new(),
                max_egress_bytes: None,
                default_permission: PermissionMode::Allow,
                visibility: ironclaw_extensions::CapabilityVisibility::Model,
                input_schema_ref: ironclaw_host_api::CapabilityProfileSchemaRef::new(
                    "schemas/fixture/input.v1.json",
                )
                .unwrap(),
                output_schema_ref: None,
                prompt_doc_ref: None,
                required_host_ports: Vec::new(),
                runtime_credentials: Vec::new(),
                resource_profile: None,
                origin_gate_matrix: None,
            }],
            hooks: Vec::new(),
        },
        VirtualPath::new(format!("/system/extensions/{service}")).unwrap(),
    )
    .unwrap()
}

struct EchoingHandler;

#[async_trait]
impl crate::FirstPartyCapabilityHandler for EchoingHandler {
    async fn dispatch(
        &self,
        request: crate::FirstPartyCapabilityRequest,
    ) -> Result<crate::FirstPartyCapabilityResult, crate::FirstPartyCapabilityError> {
        Ok(crate::FirstPartyCapabilityResult::new(
            serde_json::json!({"echoed": request.input}),
            ResourceUsage::default(),
        ))
    }
}

struct GatingHandler;

#[async_trait]
impl crate::FirstPartyCapabilityHandler for GatingHandler {
    async fn dispatch(
        &self,
        _request: crate::FirstPartyCapabilityRequest,
    ) -> Result<crate::FirstPartyCapabilityResult, crate::FirstPartyCapabilityError> {
        Err(crate::FirstPartyCapabilityError::auth_required_with(vec![
            SecretHandle::new("fixture_token").unwrap(),
        ]))
    }
}

fn binder_services(
    handlers: FirstPartyCapabilityRegistry,
) -> HostRuntimeServices<
    DiskFilesystem,
    InMemoryResourceGovernor,
    ironclaw_processes::ProcessStore<ironclaw_filesystem::InMemoryBackend>,
    ironclaw_processes::ProcessResultStore<ironclaw_filesystem::InMemoryBackend>,
> {
    HostRuntimeServices::new(
        Arc::new(ExtensionRegistry::new()),
        Arc::new(DiskFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::in_memory_backed_process_services(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_first_party_capabilities(Arc::new(handlers))
}

fn call(capability_id: &str, input: Value) -> ToolCall {
    ToolCall {
        capability_id: CapabilityId::new(capability_id).unwrap(),
        scope: sample_scope(),
        input,
        deadline: None,
        resources: ToolCallResources::default(),
    }
}

#[tokio::test]
async fn binder_routes_by_capability_id_through_the_first_party_lane() {
    let package = first_party_test_package("acme-fixture", "acme-fixture.echo");
    let services = binder_services(FirstPartyCapabilityRegistry::new().with_handler(
        CapabilityId::new("acme-fixture.echo").unwrap(),
        Arc::new(EchoingHandler),
    ));
    let binder = services.extension_lane_tool_binder();

    let adapter = binder.bind_package(Arc::new(package)).expect("binds");
    let ports = ToolPorts { egress: None };

    let result = adapter
        .invoke(
            call("acme-fixture.echo", serde_json::json!({"n": 1})),
            &ports,
        )
        .await
        .expect("lane invocation succeeds");
    assert_eq!(result.output, serde_json::json!({"echoed": {"n": 1}}));
    assert!(result.output_bytes > 0);

    let undeclared = adapter
        .invoke(call("acme-fixture.other", serde_json::json!({})), &ports)
        .await
        .unwrap_err();
    assert!(
        matches!(
            undeclared,
            ToolError::Failed {
                kind: RuntimeDispatchErrorKind::UndeclaredCapability,
                ..
            }
        ),
        "unknown capability inside a bound extension fails before lane work: {undeclared:?}"
    );
}

#[tokio::test]
async fn binder_preserves_the_auth_gate_payload_across_the_tool_abi() {
    let package = first_party_test_package("acme-gated", "acme-gated.locked");
    let services = binder_services(FirstPartyCapabilityRegistry::new().with_handler(
        CapabilityId::new("acme-gated.locked").unwrap(),
        Arc::new(GatingHandler),
    ));
    let binder = services.extension_lane_tool_binder();

    let adapter = binder
        .bind_package(Arc::new(package))
        .expect("binds gated package");
    let err = adapter
        .invoke(
            call("acme-gated.locked", serde_json::json!({})),
            &ToolPorts { egress: None },
        )
        .await
        .unwrap_err();

    match err {
        ToolError::AuthRequired {
            required_secrets, ..
        } => assert_eq!(
            required_secrets,
            vec![SecretHandle::new("fixture_token").unwrap()]
        ),
        other => panic!("expected AuthRequired, got {other:?}"),
    }
}

#[tokio::test]
async fn binder_fails_typed_for_an_unconfigured_lane() {
    // No MCP runtime configured: binding an MCP-runtime package fails with
    // the preserved missing-backend error, at bind time.
    let services = binder_services(FirstPartyCapabilityRegistry::new());
    let binder = services.extension_lane_tool_binder();
    let package = test_package(MCP_TEST_MANIFEST, "test-mcp");

    let err = match binder.bind_package(Arc::new(package)) {
        Ok(_) => panic!("binding an MCP package without an MCP lane must fail"),
        Err(err) => err,
    };
    assert_eq!(
        err,
        ExtensionToolBindError::MissingRuntimeBackend {
            runtime: RuntimeKind::Mcp
        }
    );
}

/// Success-returning MCP executor double. Records the capability id + input it
/// was handed and returns a fixed output, so the test can prove a *discovered*
/// MCP capability was routed by id, through the ToolAdapter, into the MCP lane,
/// and its output flowed back — the counterpart of `AuthRequiredMcpExecutor`
/// (which only proves the auth-gate mapping).
struct RecordingMcpExecutor {
    output: Value,
    invoked: std::sync::Mutex<Option<(CapabilityId, Value)>>,
}

#[async_trait]
impl ironclaw_mcp::McpExecutor for RecordingMcpExecutor {
    async fn execute_extension_json(
        &self,
        _governor: &dyn ResourceGovernor,
        request: ironclaw_mcp::McpExecutionRequest<'_>,
    ) -> Result<ironclaw_mcp::McpExecutionResult, ironclaw_mcp::McpError> {
        *self.invoked.lock().expect("invoked lock") = Some((
            request.capability_id.clone(),
            request.invocation.input.clone(),
        ));
        let output_bytes = serde_json::to_vec(&self.output)
            .map(|bytes| bytes.len() as u64)
            .unwrap_or_default();
        let reservation_id = ResourceReservationId::new();
        Ok(ironclaw_mcp::McpExecutionResult {
            result: ironclaw_host_api::CapabilityHostResult {
                output: self.output.clone(),
                reservation_id,
                usage: ResourceUsage::default(),
                output_bytes,
            },
            receipt: ResourceReceipt {
                id: reservation_id,
                scope: request.scope.clone(),
                status: ReservationStatus::Released,
                estimate: ResourceEstimate::default(),
                actual: None,
            },
        })
    }
}

/// A runtime HTTP egress that only needs to *exist*: a discovered MCP tool
/// carries the `network` effect, so `ServiceResolvedRuntimeAdapter` resolves an
/// egress for the plan before delegating to the executor. The `McpExecutor`
/// double returns its result without touching the network, so this egress is
/// never actually called — it errors loudly if it ever is.
struct UnusedRuntimeHttpEgress;

#[async_trait]
impl ironclaw_host_api::RuntimeHttpEgress for UnusedRuntimeHttpEgress {
    async fn execute(
        &self,
        request: ironclaw_host_api::RuntimeHttpEgressRequest,
    ) -> Result<
        ironclaw_host_api::RuntimeHttpEgressResponse,
        ironclaw_host_api::RuntimeHttpEgressError,
    > {
        Err(ironclaw_host_api::RuntimeHttpEgressError::Request {
            reason: "runtime egress must not be called in the binder routing test".to_string(),
            request_bytes: request.body.len() as u64,
            response_bytes: 0,
        })
    }
}

#[tokio::test]
async fn binder_invokes_a_discovered_mcp_tool_through_the_tool_adapter() {
    // TOOL-6 (MCP half): a hosted-MCP package whose tools are *discovered*
    // (tools/list-originated via `package_with_discovered_hosted_mcp_tools`),
    // not statically declared, binds and invokes through the very same
    // `LaneBackedToolAdapter` as the static/WASM lanes. The binder never sees
    // "discovered vs declared" — a discovered capability carries
    // `RuntimeKind::Mcp`, so it resolves the MCP lane and dispatches identically.
    let base = test_package(MCP_TEST_MANIFEST, "test-mcp");
    let discovered = ironclaw_extensions::package_with_discovered_hosted_mcp_tools(
        &base,
        &[ironclaw_extensions::HostedMcpDiscoveredTool {
            name: "search".to_string(),
            description: "Discovered search tool".to_string(),
            input_schema: serde_json::json!({"type": "object"}),
            annotations: ironclaw_extensions::HostedMcpDiscoveredToolAnnotations {
                read_only_hint: true,
                ..Default::default()
            },
        }],
    )
    .expect("discovered hosted-MCP package builds");

    let executor = Arc::new(RecordingMcpExecutor {
        output: serde_json::json!({"hits": ["a", "b"]}),
        invoked: std::sync::Mutex::new(None),
    });
    // Discovered MCP tools carry the `network` effect, so the dispatch planner
    // needs a non-deny network mode (the executor double itself never touches
    // the network — this only clears the pre-dispatch policy gate).
    let services = binder_services(FirstPartyCapabilityRegistry::new())
        .with_runtime_policy(policy_with(
            FilesystemBackendKind::HostWorkspace,
            ProcessBackendKind::LocalHost,
            NetworkMode::DirectLogged,
            SecretMode::ScrubbedEnv,
        ))
        .with_runtime_http_egress(Arc::new(UnusedRuntimeHttpEgress))
        .with_mcp_runtime(Arc::clone(&executor));
    let binder = services.extension_lane_tool_binder();

    let adapter = binder
        .bind_package(Arc::new(discovered))
        .expect("discovered MCP package binds through the MCP lane");
    let ports = ToolPorts { egress: None };

    let result = adapter
        .invoke(
            call("test-mcp.search", serde_json::json!({"query": "hi"})),
            &ports,
        )
        .await
        .expect("discovered MCP capability dispatches through the ToolAdapter");
    assert_eq!(result.output, serde_json::json!({"hits": ["a", "b"]}));
    assert!(result.output_bytes > 0);

    // The exact discovered capability id + input reached the MCP lane — routing
    // by capability id, not a static-vs-discovered code path.
    let invoked = executor
        .invoked
        .lock()
        .expect("invoked lock")
        .clone()
        .expect("the MCP lane received the discovered invocation");
    assert_eq!(invoked.0, CapabilityId::new("test-mcp.search").unwrap());
    assert_eq!(invoked.1, serde_json::json!({"query": "hi"}));
}

#[tokio::test]
async fn registry_resolver_allowlist_restricts_to_builtin_provider() {
    use ironclaw_dispatcher::ToolResolver;
    use ironclaw_extensions::SharedExtensionRegistry;

    let mut registry = ExtensionRegistry::new();
    registry
        .insert(crate::first_party_tools::builtin_first_party_package().unwrap())
        .unwrap();
    registry
        .insert(test_package(WASM_MANIFEST, "test-wasm"))
        .unwrap();
    let registry = Arc::new(SharedExtensionRegistry::new(registry));
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let allowlist: BTreeSet<ExtensionId> =
        [ExtensionId::new("builtin").unwrap()].into_iter().collect();
    let resolver = super::super::tool_resolver::RegistryLaneToolResolver::new(
        registry,
        Arc::new(RuntimeLaneExecutor::new(None, None, None, None)),
        Arc::new(DiskFilesystem::new()),
        governor,
        policy_with(
            FilesystemBackendKind::HostWorkspace,
            ProcessBackendKind::LocalHost,
            NetworkMode::DirectLogged,
            SecretMode::ScrubbedEnv,
        ),
        Some(allowlist),
    );

    assert!(
        resolver
            .resolve(&CapabilityId::new("builtin.echo").unwrap())
            .is_some(),
        "built-ins keep resolving through the registry lane"
    );
    assert!(
        resolver
            .resolve(&CapabilityId::new("test-wasm.run").unwrap())
            .is_none(),
        "extension capabilities must not resolve from the restricted registry lane"
    );
}

const MCP_TEST_MANIFEST: &str = r#"schema_version = "reborn.extension_manifest.v2"
id = "test-mcp"
name = "Test MCP"
version = "0.1.0"
description = "MCP binder fixture"
trust = "untrusted"

[runtime]
kind = "mcp"
transport = "http"
url = "https://mcp.fixture.example/mcp"

[[host_api]]
id = "ironclaw.capability_provider/v1"
section = "capability_provider.tools"

[capability_provider.tools]

[[capability_provider.tools.capabilities]]
id = "test-mcp.probe"
description = "Probe MCP"
effects = ["network"]
default_permission = "allow"
visibility = "model"
input_schema_ref = "schemas/test-mcp/probe.input.v1.json"
output_schema_ref = "schemas/test-mcp/probe.output.v1.json"
prompt_doc_ref = "prompts/test-mcp/probe.md"
"#;
