//! Extension tool binder pins (TOOL-6's lane leg, LIFE-3's missing-lane
//! error): a package prebinds to its configured lane as one behavior-only
//! [`ToolAdapter`] per extension, routing internally by capability id, with
//! the auth gate payload preserved across the ABI.

use std::collections::BTreeSet;

use ironclaw_host_api::{
    InvocationId, RequestedTrustClass, ToolCall, ToolCallResources, ToolError, ToolPorts,
    TrustClass, VirtualPath,
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
                implements: Vec::new(),
                description: "binder fixture capability".to_string(),
                effects: vec![EffectKind::DispatchCapability],
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
    LocalFilesystem,
    InMemoryResourceGovernor,
    InMemoryProcessStore,
    InMemoryProcessResultStore,
> {
    HostRuntimeServices::new(
        Arc::new(ExtensionRegistry::new()),
        Arc::new(LocalFilesystem::new()),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("surface-v1").unwrap(),
    )
    .with_first_party_capabilities(Arc::new(handlers))
}

fn call(capability_id: &str, input: Value) -> ToolCall {
    ToolCall {
        capability_id: CapabilityId::new(capability_id).unwrap(),
        invocation_id: InvocationId::new(),
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
    let ports = ToolPorts {
        egress: None,
        state: None,
    };

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
            &ToolPorts {
                egress: None,
                state: None,
            },
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
        std::collections::HashMap::<
            RuntimeKind,
            Arc<dyn super::super::runtime_adapters::RuntimeAdapter<_, _>>,
        >::new(),
        Arc::new(LocalFilesystem::new()),
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
