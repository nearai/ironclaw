use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use async_trait::async_trait;
use ironclaw_authorization::CapabilityDispatchAuthorizer;
use ironclaw_capabilities::CapabilityInvocationRequest;
use ironclaw_events::InMemoryAuditSink;
use ironclaw_extensions::{ExtensionManifest, ExtensionPackage, ExtensionRegistry};
use ironclaw_filesystem::LocalFilesystem;
use ironclaw_host_api::*;
use ironclaw_host_runtime::HostRuntimeServices;
use ironclaw_processes::ProcessServices;
use ironclaw_resources::InMemoryResourceGovernor;
use ironclaw_scripts::{
    ScriptBackend, ScriptBackendOutput, ScriptBackendRequest, ScriptRuntime, ScriptRuntimeConfig,
};
use ironclaw_wasm::{WasmHostHttp, WasmHttpRequest, WasmHttpResponse, WasmRuntime};
use serde_json::json;

#[tokio::test]
async fn apply_network_policy_obligation_is_enforced_by_wasm_network_imports() {
    let client = RecordingHttpClient::new(WasmHttpResponse {
        status: 200,
        body: r#"{"ok":true}"#.to_string(),
    });
    let (filesystem, package) =
        wasm_package_with_module(http_module_bytes("https://api.example.test/v1/echo", 0, ""));
    let registry = Arc::new(registry_with_package(package));
    let filesystem = Arc::new(filesystem);
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let authorizer = Arc::new(NetworkPolicyAuthorizer::new(NetworkPolicy {
        allowed_targets: vec![NetworkTargetPattern {
            scheme: Some(NetworkScheme::Https),
            host_pattern: "api.example.test".to_string(),
            port: None,
        }],
        deny_private_ip_ranges: true,
        max_egress_bytes: Some(1024),
    }));
    let services = HostRuntimeServices::new(
        registry,
        filesystem,
        governor,
        authorizer,
        ProcessServices::in_memory(),
    )
    .with_wasm_runtime(Arc::new(WasmRuntime::for_testing().unwrap()))
    .with_wasm_http_client(Arc::new(client.clone()))
    .with_audit_sink(Arc::new(InMemoryAuditSink::new()))
    .with_builtin_obligation_handler();
    let dispatcher = services.runtime_dispatcher_arc();
    let capability_host = services.capability_host_for_runtime_dispatcher(&dispatcher);

    let result = capability_host
        .invoke_json(CapabilityInvocationRequest {
            context: execution_context(),
            capability_id: CapabilityId::new("net-demo.http").unwrap(),
            estimate: ResourceEstimate::default(),
            input: json!({}),
        })
        .await
        .unwrap();

    assert_eq!(result.dispatch.output, json!({"ok": true}));
    let requests = client.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].url, "https://api.example.test/v1/echo");
}

#[tokio::test]
async fn apply_network_policy_obligation_blocks_wasm_network_import_before_client_call() {
    let client = RecordingHttpClient::new(WasmHttpResponse {
        status: 200,
        body: r#"{"ok":true}"#.to_string(),
    });
    let (filesystem, package) = wasm_package_with_module(http_module_bytes(
        "https://blocked.example.test/v1/echo",
        0,
        "",
    ));
    let registry = Arc::new(registry_with_package(package));
    let filesystem = Arc::new(filesystem);
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let authorizer = Arc::new(NetworkPolicyAuthorizer::new(NetworkPolicy {
        allowed_targets: vec![NetworkTargetPattern {
            scheme: Some(NetworkScheme::Https),
            host_pattern: "api.example.test".to_string(),
            port: None,
        }],
        deny_private_ip_ranges: true,
        max_egress_bytes: Some(1024),
    }));
    let services = HostRuntimeServices::new(
        registry,
        filesystem,
        governor,
        authorizer,
        ProcessServices::in_memory(),
    )
    .with_wasm_runtime(Arc::new(WasmRuntime::for_testing().unwrap()))
    .with_wasm_http_client(Arc::new(client.clone()))
    .with_audit_sink(Arc::new(InMemoryAuditSink::new()))
    .with_builtin_obligation_handler();
    let dispatcher = services.runtime_dispatcher_arc();
    let capability_host = services.capability_host_for_runtime_dispatcher(&dispatcher);

    let result = capability_host
        .invoke_json(CapabilityInvocationRequest {
            context: execution_context(),
            capability_id: CapabilityId::new("net-demo.http").unwrap(),
            estimate: ResourceEstimate::default(),
            input: json!({}),
        })
        .await
        .unwrap();

    assert_eq!(result.dispatch.output, json!({"ok": false}));
    assert!(client.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn apply_network_policy_obligation_fails_closed_for_script_runtime_without_network_plumbing()
{
    let called = Arc::new(AtomicBool::new(false));
    let registry = Arc::new(registry_with_package(script_package()));
    let filesystem = Arc::new(LocalFilesystem::new());
    let governor = Arc::new(InMemoryResourceGovernor::new());
    let authorizer = Arc::new(NetworkPolicyAuthorizer::new(NetworkPolicy {
        allowed_targets: vec![NetworkTargetPattern {
            scheme: Some(NetworkScheme::Https),
            host_pattern: "api.example.test".to_string(),
            port: None,
        }],
        deny_private_ip_ranges: true,
        max_egress_bytes: Some(1024),
    }));
    let services = HostRuntimeServices::new(
        registry,
        filesystem,
        governor,
        authorizer,
        ProcessServices::in_memory(),
    )
    .with_script_runtime(Arc::new(ScriptRuntime::new(
        ScriptRuntimeConfig::for_testing(),
        RecordingScriptBackend {
            called: Arc::clone(&called),
        },
    )))
    .with_builtin_obligation_handler();
    let dispatcher = services.runtime_dispatcher_arc();
    let capability_host = services.capability_host_for_runtime_dispatcher(&dispatcher);

    let err = capability_host
        .invoke_json(CapabilityInvocationRequest {
            context: execution_context(),
            capability_id: CapabilityId::new("script-net.http").unwrap(),
            estimate: ResourceEstimate::default(),
            input: json!({}),
        })
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        ironclaw_capabilities::CapabilityInvocationError::Dispatch { kind }
            if kind == "NetworkDenied"
    ));
    assert!(!called.load(Ordering::SeqCst));
}

#[derive(Clone)]
struct RecordingHttpClient {
    response: WasmHttpResponse,
    requests: Arc<Mutex<Vec<WasmHttpRequest>>>,
}

impl RecordingHttpClient {
    fn new(response: WasmHttpResponse) -> Self {
        Self {
            response,
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl WasmHostHttp for RecordingHttpClient {
    fn request_utf8(&self, request: WasmHttpRequest) -> Result<WasmHttpResponse, String> {
        self.requests.lock().unwrap().push(request);
        Ok(self.response.clone())
    }
}

struct RecordingScriptBackend {
    called: Arc<AtomicBool>,
}

impl ScriptBackend for RecordingScriptBackend {
    fn execute(&self, _request: ScriptBackendRequest) -> Result<ScriptBackendOutput, String> {
        self.called.store(true, Ordering::SeqCst);
        Ok(ScriptBackendOutput::json(json!({"ok": true})))
    }
}

struct NetworkPolicyAuthorizer {
    policy: NetworkPolicy,
}

impl NetworkPolicyAuthorizer {
    fn new(policy: NetworkPolicy) -> Self {
        Self { policy }
    }
}

#[async_trait]
impl CapabilityDispatchAuthorizer for NetworkPolicyAuthorizer {
    async fn authorize_dispatch(
        &self,
        _context: &ExecutionContext,
        _descriptor: &CapabilityDescriptor,
        _estimate: &ResourceEstimate,
    ) -> Decision {
        Decision::Allow {
            obligations: vec![Obligation::ApplyNetworkPolicy {
                policy: self.policy.clone(),
            }],
        }
    }

    async fn authorize_spawn(
        &self,
        _context: &ExecutionContext,
        _descriptor: &CapabilityDescriptor,
        _estimate: &ResourceEstimate,
    ) -> Decision {
        Decision::Allow {
            obligations: vec![Obligation::ApplyNetworkPolicy {
                policy: self.policy.clone(),
            }],
        }
    }
}

fn wasm_package_with_module(bytes: Vec<u8>) -> (LocalFilesystem, ExtensionPackage) {
    let storage = tempfile::tempdir().unwrap().keep();
    std::fs::create_dir_all(storage.join("net-demo/wasm")).unwrap();
    std::fs::write(storage.join("net-demo/wasm/net_demo.wasm"), bytes).unwrap();

    let mut fs = LocalFilesystem::new();
    fs.mount_local(
        VirtualPath::new("/system/extensions").unwrap(),
        HostPath::from_path_buf(storage),
    )
    .unwrap();

    let package = ExtensionPackage::from_manifest(
        ExtensionManifest::parse(WASM_MANIFEST).unwrap(),
        VirtualPath::new("/system/extensions/net-demo").unwrap(),
    )
    .unwrap();
    (fs, package)
}

fn registry_with_package(package: ExtensionPackage) -> ExtensionRegistry {
    let mut registry = ExtensionRegistry::new();
    registry.insert(package).unwrap();
    registry
}

fn script_package() -> ExtensionPackage {
    ExtensionPackage::from_manifest(
        ExtensionManifest::parse(SCRIPT_MANIFEST).unwrap(),
        VirtualPath::new("/system/extensions/script-net").unwrap(),
    )
    .unwrap()
}

fn execution_context() -> ExecutionContext {
    let invocation_id = InvocationId::new();
    let resource_scope = ResourceScope {
        tenant_id: TenantId::new("tenant1").unwrap(),
        user_id: UserId::new("user1").unwrap(),
        project_id: Some(ProjectId::new("project1").unwrap()),
        mission_id: None,
        thread_id: None,
        invocation_id,
    };
    ExecutionContext {
        invocation_id,
        correlation_id: CorrelationId::new(),
        process_id: None,
        parent_process_id: None,
        tenant_id: resource_scope.tenant_id.clone(),
        user_id: resource_scope.user_id.clone(),
        project_id: resource_scope.project_id.clone(),
        mission_id: resource_scope.mission_id.clone(),
        thread_id: resource_scope.thread_id.clone(),
        extension_id: ExtensionId::new("caller").unwrap(),
        runtime: RuntimeKind::Wasm,
        trust: TrustClass::Sandbox,
        grants: CapabilitySet::default(),
        mounts: MountView::default(),
        resource_scope,
    }
}

fn http_module_bytes(url: &str, method: i32, body: &str) -> Vec<u8> {
    let url_len = url.len();
    let body_len = body.len();
    wat::parse_str(format!(
        r#"(module
            (import "host" "http_request_utf8" (func $http (param i32 i32 i32 i32 i32 i32 i32) (result i32)))
            (memory (export "memory") 1)
            (data (i32.const 64) "{url}")
            (data (i32.const 256) "{{\"ok\":false}}")
            (data (i32.const 512) "{body}")
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
            (func (export "http") (param i32 i32) (result i32)
              (local $n i32)
              i32.const {method}
              i32.const 64
              i32.const {url_len}
              i32.const 512
              i32.const {body_len}
              i32.const 4096
              i32.const 512
              call $http
              local.set $n
              local.get $n
              i32.const 0
              i32.ge_s
              if
                i32.const 4096
                global.set $out_ptr
                local.get $n
                global.set $out_len
              else
                i32.const 256
                global.set $out_ptr
                i32.const 12
                global.set $out_len
              end
              i32.const 0)
            (func (export "output_ptr") (result i32) global.get $out_ptr)
            (func (export "output_len") (result i32) global.get $out_len))"#
    ))
    .unwrap()
}

const WASM_MANIFEST: &str = r#"
id = "net-demo"
name = "Network Demo"
version = "0.1.0"
description = "Network demo extension"
trust = "sandbox"

[runtime]
kind = "wasm"
module = "wasm/net_demo.wasm"

[[capabilities]]
id = "net-demo.http"
description = "HTTP demo"
effects = ["network", "dispatch_capability"]
default_permission = "allow"
parameters_schema = { type = "object" }
"#;

const SCRIPT_MANIFEST: &str = r#"
id = "script-net"
name = "Script Net"
version = "0.1.0"
description = "Script network demo extension"
trust = "sandbox"

[runtime]
kind = "script"
runner = "sandboxed_process"
command = "echo"

[[capabilities]]
id = "script-net.http"
description = "HTTP demo"
effects = ["network", "dispatch_capability"]
default_permission = "allow"
parameters_schema = { type = "object" }
"#;
