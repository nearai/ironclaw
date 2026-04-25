use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ironclaw_dispatcher::*;
use ironclaw_extensions::*;
use ironclaw_filesystem::*;
use ironclaw_host_api::*;
use ironclaw_mcp::*;
use ironclaw_resources::*;
use ironclaw_scripts::*;
use ironclaw_wasm::*;
use serde_json::json;

#[tokio::test]
async fn dispatcher_routes_wasm_capability_through_real_wasm_executor() {
    let (fs, package) = wasm_package_with_module(json_echo_module());
    let mut registry = ExtensionRegistry::new();
    registry.insert(package).unwrap();
    let runtime = WasmRuntime::for_testing().unwrap();
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope();
    let account = ResourceAccount::tenant(scope.tenant_id.clone());
    governor.set_limit(
        account.clone(),
        ResourceLimits {
            max_concurrency_slots: Some(1),
            max_output_bytes: Some(10_000),
            ..ResourceLimits::default()
        },
    );

    let dispatcher = RuntimeDispatcher::new(&registry, &fs, &governor).with_wasm_runtime(&runtime);
    let result = dispatcher
        .dispatch_json(CapabilityDispatchRequest {
            capability_id: CapabilityId::new("echo.say").unwrap(),
            scope,
            estimate: ResourceEstimate {
                concurrency_slots: Some(1),
                output_bytes: Some(10_000),
                ..ResourceEstimate::default()
            },
            input: json!({"message": "hello kernel"}),
        })
        .await
        .unwrap();

    assert_eq!(result.capability_id, CapabilityId::new("echo.say").unwrap());
    assert_eq!(result.provider, ExtensionId::new("echo").unwrap());
    assert_eq!(result.runtime, RuntimeKind::Wasm);
    assert_eq!(result.output, json!({"message": "hello kernel"}));
    assert_eq!(result.receipt.status, ReservationStatus::Reconciled);
    assert_eq!(governor.reserved_for(&account), ResourceTally::default());
    assert!(governor.usage_for(&account).output_bytes > 0);
}

#[tokio::test]
async fn dispatcher_routes_script_capability_through_script_runtime() {
    let fs = mounted_empty_extension_root();
    let mut registry = ExtensionRegistry::new();
    registry
        .insert(package_from_manifest(SCRIPT_MANIFEST))
        .unwrap();
    let backend = RecordingScriptBackend::new(ScriptBackendOutput::json(json!({
        "message": "hello script kernel"
    })));
    let script_runtime = ScriptRuntime::new(ScriptRuntimeConfig::for_testing(), backend.clone());
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope();
    let account = ResourceAccount::tenant(scope.tenant_id.clone());
    governor.set_limit(
        account.clone(),
        ResourceLimits {
            max_concurrency_slots: Some(1),
            max_process_count: Some(10),
            max_output_bytes: Some(10_000),
            ..ResourceLimits::default()
        },
    );

    let dispatcher =
        RuntimeDispatcher::new(&registry, &fs, &governor).with_script_runtime(&script_runtime);
    let result = dispatcher
        .dispatch_json(CapabilityDispatchRequest {
            capability_id: CapabilityId::new("script.echo").unwrap(),
            scope,
            estimate: ResourceEstimate {
                concurrency_slots: Some(1),
                process_count: Some(1),
                output_bytes: Some(10_000),
                ..ResourceEstimate::default()
            },
            input: json!({"message": "hello script kernel"}),
        })
        .await
        .unwrap();

    assert_eq!(
        result.capability_id,
        CapabilityId::new("script.echo").unwrap()
    );
    assert_eq!(result.provider, ExtensionId::new("script").unwrap());
    assert_eq!(result.runtime, RuntimeKind::Script);
    assert_eq!(result.output, json!({"message": "hello script kernel"}));
    assert_eq!(result.receipt.status, ReservationStatus::Reconciled);
    assert_eq!(governor.reserved_for(&account), ResourceTally::default());
    assert_eq!(governor.usage_for(&account).process_count, 1);

    let requests = backend.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].command, "script-echo");
    assert_eq!(requests[0].args, vec!["--json".to_string()]);
}

#[tokio::test]
async fn dispatcher_routes_mcp_capability_through_mcp_runtime() {
    let fs = mounted_empty_extension_root();
    let mut registry = ExtensionRegistry::new();
    registry
        .insert(package_from_manifest(MCP_MANIFEST))
        .unwrap();
    let client = RecordingMcpClient::new(McpClientOutput::json(json!({
        "matches": ["ironclaw"]
    })));
    let mcp_runtime = McpRuntime::new(McpRuntimeConfig::for_testing(), client.clone());
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope();
    let account = ResourceAccount::tenant(scope.tenant_id.clone());
    governor.set_limit(
        account.clone(),
        ResourceLimits {
            max_concurrency_slots: Some(1),
            max_process_count: Some(1),
            max_output_bytes: Some(10_000),
            ..ResourceLimits::default()
        },
    );

    let dispatcher =
        RuntimeDispatcher::new(&registry, &fs, &governor).with_mcp_runtime(&mcp_runtime);
    let result = dispatcher
        .dispatch_json(CapabilityDispatchRequest {
            capability_id: CapabilityId::new("github-mcp.search").unwrap(),
            scope,
            estimate: ResourceEstimate {
                concurrency_slots: Some(1),
                process_count: Some(1),
                output_bytes: Some(10_000),
                ..ResourceEstimate::default()
            },
            input: json!({"query": "ironclaw"}),
        })
        .await
        .unwrap();

    assert_eq!(
        result.capability_id,
        CapabilityId::new("github-mcp.search").unwrap()
    );
    assert_eq!(result.provider, ExtensionId::new("github-mcp").unwrap());
    assert_eq!(result.runtime, RuntimeKind::Mcp);
    assert_eq!(result.output, json!({"matches": ["ironclaw"]}));
    assert_eq!(result.receipt.status, ReservationStatus::Reconciled);
    assert_eq!(governor.reserved_for(&account), ResourceTally::default());
    assert!(governor.usage_for(&account).output_bytes > 0);

    let requests = client.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].transport, "stdio");
    assert_eq!(requests[0].command.as_deref(), Some("github-mcp"));
    assert_eq!(requests[0].input, json!({"query": "ironclaw"}));
}

#[tokio::test]
async fn dispatcher_fails_unknown_capability_without_reserving_resources() {
    let fs = mounted_empty_extension_root();
    let registry = ExtensionRegistry::new();
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope();
    let account = ResourceAccount::tenant(scope.tenant_id.clone());
    let runtime = WasmRuntime::for_testing().unwrap();

    let dispatcher = RuntimeDispatcher::new(&registry, &fs, &governor).with_wasm_runtime(&runtime);
    let err = dispatcher
        .dispatch_json(CapabilityDispatchRequest {
            capability_id: CapabilityId::new("missing.say").unwrap(),
            scope,
            estimate: ResourceEstimate {
                concurrency_slots: Some(1),
                ..ResourceEstimate::default()
            },
            input: json!({"message": "nope"}),
        })
        .await
        .unwrap_err();

    assert!(matches!(err, DispatchError::UnknownCapability { .. }));
    assert_eq!(governor.reserved_for(&account), ResourceTally::default());
    assert_eq!(governor.usage_for(&account), ResourceTally::default());
}

#[tokio::test]
async fn dispatcher_fails_closed_when_descriptor_runtime_does_not_match_package_runtime() {
    let (fs, mut package) = wasm_package_with_module(json_echo_module());
    package.capabilities[0].runtime = RuntimeKind::Script;
    let mut registry = ExtensionRegistry::new();
    registry.insert(package).unwrap();
    let runtime = WasmRuntime::for_testing().unwrap();
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope();
    let account = ResourceAccount::tenant(scope.tenant_id.clone());

    let dispatcher = RuntimeDispatcher::new(&registry, &fs, &governor).with_wasm_runtime(&runtime);
    let err = dispatcher
        .dispatch_json(CapabilityDispatchRequest {
            capability_id: CapabilityId::new("echo.say").unwrap(),
            scope,
            estimate: ResourceEstimate {
                concurrency_slots: Some(1),
                ..ResourceEstimate::default()
            },
            input: json!({"message": "blocked"}),
        })
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        DispatchError::RuntimeMismatch {
            descriptor_runtime: RuntimeKind::Script,
            package_runtime: RuntimeKind::Wasm,
            ..
        }
    ));
    assert_eq!(governor.reserved_for(&account), ResourceTally::default());
    assert_eq!(governor.usage_for(&account), ResourceTally::default());
}

#[tokio::test]
async fn dispatcher_recognizes_host_lanes_but_does_not_execute_without_backends() {
    for (manifest, capability) in [
        (FIRST_PARTY_MANIFEST, "conversation.ingest"),
        (SYSTEM_MANIFEST, "system.audit"),
    ] {
        let fs = mounted_empty_extension_root();
        let package = package_from_manifest(manifest);
        let runtime = package.manifest.runtime_kind();
        let mut registry = ExtensionRegistry::new();
        registry.insert(package).unwrap();
        let governor = InMemoryResourceGovernor::new();
        let scope = sample_scope();
        let account = ResourceAccount::tenant(scope.tenant_id.clone());

        let dispatcher = RuntimeDispatcher::new(&registry, &fs, &governor);
        let err = dispatcher
            .dispatch_json(CapabilityDispatchRequest {
                capability_id: CapabilityId::new(capability).unwrap(),
                scope,
                estimate: ResourceEstimate {
                    concurrency_slots: Some(1),
                    ..ResourceEstimate::default()
                },
                input: json!({}),
            })
            .await
            .unwrap_err();

        assert!(matches!(
            err,
            DispatchError::UnsupportedRuntime { runtime: actual, .. } if actual == runtime
        ));
        assert_eq!(governor.reserved_for(&account), ResourceTally::default());
        assert_eq!(governor.usage_for(&account), ResourceTally::default());
    }
}

#[tokio::test]
async fn dispatcher_requires_mcp_backend_before_reserving_resources() {
    let fs = mounted_empty_extension_root();
    let mut registry = ExtensionRegistry::new();
    registry
        .insert(package_from_manifest(MCP_MANIFEST))
        .unwrap();
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope();
    let account = ResourceAccount::tenant(scope.tenant_id.clone());

    let dispatcher = RuntimeDispatcher::new(&registry, &fs, &governor);
    let err = dispatcher
        .dispatch_json(CapabilityDispatchRequest {
            capability_id: CapabilityId::new("github-mcp.search").unwrap(),
            scope,
            estimate: ResourceEstimate {
                concurrency_slots: Some(1),
                process_count: Some(1),
                ..ResourceEstimate::default()
            },
            input: json!({"query": "blocked"}),
        })
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        DispatchError::MissingRuntimeBackend {
            runtime: RuntimeKind::Mcp
        }
    ));
    assert_eq!(governor.reserved_for(&account), ResourceTally::default());
    assert_eq!(governor.usage_for(&account), ResourceTally::default());
}

#[tokio::test]
async fn dispatcher_requires_script_backend_before_reserving_resources() {
    let fs = mounted_empty_extension_root();
    let mut registry = ExtensionRegistry::new();
    registry
        .insert(package_from_manifest(SCRIPT_MANIFEST))
        .unwrap();
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope();
    let account = ResourceAccount::tenant(scope.tenant_id.clone());

    let dispatcher = RuntimeDispatcher::new(&registry, &fs, &governor);
    let err = dispatcher
        .dispatch_json(CapabilityDispatchRequest {
            capability_id: CapabilityId::new("script.echo").unwrap(),
            scope,
            estimate: ResourceEstimate {
                concurrency_slots: Some(1),
                process_count: Some(1),
                ..ResourceEstimate::default()
            },
            input: json!({"message": "blocked"}),
        })
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        DispatchError::MissingRuntimeBackend {
            runtime: RuntimeKind::Script
        }
    ));
    assert_eq!(governor.reserved_for(&account), ResourceTally::default());
    assert_eq!(governor.usage_for(&account), ResourceTally::default());
}

#[tokio::test]
async fn dispatcher_requires_wasm_backend_before_reserving_resources() {
    let (fs, package) = wasm_package_with_module(json_echo_module());
    let mut registry = ExtensionRegistry::new();
    registry.insert(package).unwrap();
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope();
    let account = ResourceAccount::tenant(scope.tenant_id.clone());

    let dispatcher = RuntimeDispatcher::new(&registry, &fs, &governor);
    let err = dispatcher
        .dispatch_json(CapabilityDispatchRequest {
            capability_id: CapabilityId::new("echo.say").unwrap(),
            scope,
            estimate: ResourceEstimate {
                concurrency_slots: Some(1),
                ..ResourceEstimate::default()
            },
            input: json!({"message": "blocked"}),
        })
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        DispatchError::MissingRuntimeBackend {
            runtime: RuntimeKind::Wasm
        }
    ));
    assert_eq!(governor.reserved_for(&account), ResourceTally::default());
    assert_eq!(governor.usage_for(&account), ResourceTally::default());
}

fn wasm_package_with_module(bytes: Vec<u8>) -> (LocalFilesystem, ExtensionPackage) {
    let storage = tempfile::tempdir().unwrap().keep();
    std::fs::create_dir_all(storage.join("echo/wasm")).unwrap();
    std::fs::write(storage.join("echo/wasm/echo.wasm"), bytes).unwrap();

    let mut fs = LocalFilesystem::new();
    fs.mount_local(
        VirtualPath::new("/system/extensions").unwrap(),
        HostPath::from_path_buf(storage),
    )
    .unwrap();
    (fs, package_from_manifest(WASM_MANIFEST))
}

fn mounted_empty_extension_root() -> LocalFilesystem {
    let storage = tempfile::tempdir().unwrap().keep();
    let mut fs = LocalFilesystem::new();
    fs.mount_local(
        VirtualPath::new("/system/extensions").unwrap(),
        HostPath::from_path_buf(storage),
    )
    .unwrap();
    fs
}

fn package_from_manifest(manifest: &str) -> ExtensionPackage {
    let manifest = ExtensionManifest::parse(manifest).unwrap();
    let root = VirtualPath::new(format!("/system/extensions/{}", manifest.id.as_str())).unwrap();
    ExtensionPackage::from_manifest(manifest, root).unwrap()
}

fn json_echo_module() -> Vec<u8> {
    wat::parse_str(
        r#"(module
            (memory (export "memory") 1)
            (global $heap (mut i32) (i32.const 1024))
            (global $out_ptr (mut i32) (i32.const 0))
            (global $out_len (mut i32) (i32.const 0))
            (func (export "alloc") (param $len i32) (result i32)
              (local $ptr i32)
              global.get $heap
              local.set $ptr
              global.get $heap
              local.get $len
              i32.add
              global.set $heap
              local.get $ptr)
            (func (export "say") (param $ptr i32) (param $len i32) (result i32)
              local.get $ptr
              global.set $out_ptr
              local.get $len
              global.set $out_len
              i32.const 0)
            (func (export "output_ptr") (result i32)
              global.get $out_ptr)
            (func (export "output_len") (result i32)
              global.get $out_len))"#,
    )
    .unwrap()
}

#[derive(Clone)]
struct RecordingMcpClient {
    output: McpClientOutput,
    requests: Arc<Mutex<Vec<McpClientRequest>>>,
}

impl RecordingMcpClient {
    fn new(output: McpClientOutput) -> Self {
        Self {
            output,
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

#[async_trait]
impl McpClient for RecordingMcpClient {
    async fn call_tool(&self, request: McpClientRequest) -> Result<McpClientOutput, String> {
        self.requests.lock().unwrap().push(request);
        Ok(self.output.clone())
    }
}

#[derive(Clone)]
struct RecordingScriptBackend {
    output: ScriptBackendOutput,
    requests: std::sync::Arc<std::sync::Mutex<Vec<ScriptBackendRequest>>>,
}

impl RecordingScriptBackend {
    fn new(output: ScriptBackendOutput) -> Self {
        Self {
            output,
            requests: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
        }
    }
}

impl ScriptBackend for RecordingScriptBackend {
    fn execute(&self, request: ScriptBackendRequest) -> Result<ScriptBackendOutput, String> {
        self.requests.lock().unwrap().push(request);
        Ok(self.output.clone())
    }
}

fn sample_scope() -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new("tenant1").unwrap(),
        user_id: UserId::new("user1").unwrap(),
        project_id: Some(ProjectId::new("project1").unwrap()),
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    }
}

const WASM_MANIFEST: &str = r#"
id = "echo"
name = "Echo"
version = "0.1.0"
description = "Echo demo extension"
trust = "sandbox"

[runtime]
kind = "wasm"
module = "wasm/echo.wasm"

[[capabilities]]
id = "echo.say"
description = "Echo text"
effects = ["dispatch_capability"]
default_permission = "allow"
parameters_schema = { type = "object", required = ["message"], properties = { message = { type = "string" } } }
"#;

const SCRIPT_MANIFEST: &str = r#"
id = "script"
name = "Script Echo"
version = "0.1.0"
description = "Script demo extension"
trust = "sandbox"

[runtime]
kind = "script"
backend = "docker"
image = "alpine:latest"
command = "script-echo"
args = ["--json"]

[[capabilities]]
id = "script.echo"
description = "Echo text"
effects = ["dispatch_capability"]
default_permission = "allow"
parameters_schema = { type = "object" }
"#;

const MCP_MANIFEST: &str = r#"
id = "github-mcp"
name = "GitHub MCP"
version = "0.1.0"
description = "GitHub MCP adapter"
trust = "sandbox"

[runtime]
kind = "mcp"
transport = "stdio"
command = "github-mcp"

[[capabilities]]
id = "github-mcp.search"
description = "Search GitHub"
effects = ["network", "dispatch_capability"]
default_permission = "ask"
parameters_schema = { type = "object" }
"#;

const FIRST_PARTY_MANIFEST: &str = r#"
id = "conversation"
name = "Conversation"
version = "0.1.0"
description = "Conversation service"
trust = "system"

[runtime]
kind = "first_party"
service = "conversation"

[[capabilities]]
id = "conversation.ingest"
description = "Ingest message"
effects = ["dispatch_capability"]
default_permission = "allow"
parameters_schema = { type = "object" }
"#;

const SYSTEM_MANIFEST: &str = r#"
id = "system"
name = "System"
version = "0.1.0"
description = "System service"
trust = "system"

[runtime]
kind = "system"
service = "audit"

[[capabilities]]
id = "system.audit"
description = "Emit audit event"
effects = ["dispatch_capability"]
default_permission = "allow"
parameters_schema = { type = "object" }
"#;
