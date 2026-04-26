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
async fn vertical_slice_discovers_and_dispatches_wasm_script_and_mcp_capabilities() {
    let fs = filesystem_with_echo_extensions();
    let registry =
        ExtensionDiscovery::discover(&fs, &VirtualPath::new("/system/extensions").unwrap())
            .await
            .unwrap();
    assert_eq!(registry.extensions().count(), 3);

    let governor = InMemoryResourceGovernor::new();
    let wasm_runtime = WasmRuntime::for_testing().unwrap();
    let script_backend = EchoScriptBackend;
    let script_runtime = ScriptRuntime::new(ScriptRuntimeConfig::for_testing(), script_backend);
    let mcp_runtime = McpRuntime::new(McpRuntimeConfig::for_testing(), EchoMcpClient);
    let scope = sample_scope();
    let dispatcher = RuntimeDispatcher::new(&registry, &fs, &governor)
        .with_wasm_runtime(&wasm_runtime)
        .with_script_runtime(&script_runtime)
        .with_mcp_runtime(&mcp_runtime);

    let wasm_scope = scope.clone();
    let wasm_account = ResourceAccount::tenant(wasm_scope.tenant_id.clone());
    let wasm = dispatcher
        .dispatch_json(CapabilityDispatchRequest {
            capability_id: CapabilityId::new("echo-wasm.say").unwrap(),
            scope: wasm_scope,
            estimate: ResourceEstimate {
                concurrency_slots: Some(1),
                output_bytes: Some(10_000),
                ..ResourceEstimate::default()
            },
            input: json!({"message": "hello wasm"}),
        })
        .await
        .unwrap();

    assert_eq!(wasm.provider, ExtensionId::new("echo-wasm").unwrap());
    assert_eq!(wasm.runtime, RuntimeKind::Wasm);
    assert_eq!(wasm.output, json!({"message": "hello wasm"}));
    assert_eq!(wasm.receipt.status, ReservationStatus::Reconciled);
    assert_eq!(
        governor.reserved_for(&wasm_account),
        ResourceTally::default()
    );

    let script_scope = scope.clone();
    let script_account = ResourceAccount::tenant(script_scope.tenant_id.clone());
    let script = dispatcher
        .dispatch_json(CapabilityDispatchRequest {
            capability_id: CapabilityId::new("echo-script.say").unwrap(),
            scope: script_scope,
            estimate: ResourceEstimate {
                concurrency_slots: Some(1),
                process_count: Some(1),
                output_bytes: Some(10_000),
                ..ResourceEstimate::default()
            },
            input: json!({"message": "hello script"}),
        })
        .await
        .unwrap();

    assert_eq!(script.provider, ExtensionId::new("echo-script").unwrap());
    assert_eq!(script.runtime, RuntimeKind::Script);
    assert_eq!(script.output, json!({"message": "hello script"}));
    assert_eq!(script.receipt.status, ReservationStatus::Reconciled);
    assert_eq!(
        governor.reserved_for(&script_account),
        ResourceTally::default()
    );
    assert_eq!(script.usage.process_count, 1);
    assert!(governor.usage_for(&script_account).process_count >= 1);

    let mcp_scope = scope;
    let mcp_account = ResourceAccount::tenant(mcp_scope.tenant_id.clone());
    let mcp = dispatcher
        .dispatch_json(CapabilityDispatchRequest {
            capability_id: CapabilityId::new("echo-mcp.say").unwrap(),
            scope: mcp_scope,
            estimate: ResourceEstimate {
                concurrency_slots: Some(1),
                process_count: Some(1),
                output_bytes: Some(10_000),
                ..ResourceEstimate::default()
            },
            input: json!({"message": "hello mcp"}),
        })
        .await
        .unwrap();

    assert_eq!(mcp.provider, ExtensionId::new("echo-mcp").unwrap());
    assert_eq!(mcp.runtime, RuntimeKind::Mcp);
    assert_eq!(mcp.output, json!({"message": "hello mcp"}));
    assert_eq!(mcp.receipt.status, ReservationStatus::Reconciled);
    assert_eq!(
        governor.reserved_for(&mcp_account),
        ResourceTally::default()
    );
    assert_eq!(mcp.usage.process_count, 1);
}

#[derive(Clone)]
struct EchoMcpClient;

#[async_trait]
impl McpClient for EchoMcpClient {
    async fn call_tool(&self, request: McpClientRequest) -> Result<McpClientOutput, String> {
        Ok(McpClientOutput::json(request.input))
    }
}

#[derive(Clone)]
struct EchoScriptBackend;

impl ScriptBackend for EchoScriptBackend {
    fn execute(&self, request: ScriptBackendRequest) -> Result<ScriptBackendOutput, String> {
        Ok(ScriptBackendOutput {
            exit_code: 0,
            stdout: request.stdin_json.into_bytes(),
            stderr: Vec::new(),
            wall_clock_ms: 1,
        })
    }
}

fn filesystem_with_echo_extensions() -> LocalFilesystem {
    let storage = tempfile::tempdir().unwrap().keep();
    let wasm_root = storage.join("echo-wasm");
    std::fs::create_dir_all(wasm_root.join("wasm")).unwrap();
    std::fs::write(wasm_root.join("manifest.toml"), WASM_MANIFEST).unwrap();
    std::fs::write(wasm_root.join("wasm/echo.wasm"), json_echo_module()).unwrap();

    let script_root = storage.join("echo-script");
    std::fs::create_dir_all(&script_root).unwrap();
    std::fs::write(script_root.join("manifest.toml"), SCRIPT_MANIFEST).unwrap();

    let mcp_root = storage.join("echo-mcp");
    std::fs::create_dir_all(&mcp_root).unwrap();
    std::fs::write(mcp_root.join("manifest.toml"), MCP_MANIFEST).unwrap();

    let mut fs = LocalFilesystem::new();
    fs.mount_local(
        VirtualPath::new("/system/extensions").unwrap(),
        HostPath::from_path_buf(storage),
    )
    .unwrap();
    fs
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
id = "echo-wasm"
name = "WASM Echo"
version = "0.1.0"
description = "WASM echo demo extension"
trust = "sandbox"

[runtime]
kind = "wasm"
module = "wasm/echo.wasm"

[[capabilities]]
id = "echo-wasm.say"
description = "Echo text through WASM"
effects = ["dispatch_capability"]
default_permission = "allow"
parameters_schema = { type = "object", required = ["message"], properties = { message = { type = "string" } } }
"#;

const MCP_MANIFEST: &str = r#"
id = "echo-mcp"
name = "MCP Echo"
version = "0.1.0"
description = "MCP echo demo adapter"
trust = "sandbox"

[runtime]
kind = "mcp"
transport = "stdio"
command = "echo-mcp"
args = ["--stdio"]

[[capabilities]]
id = "echo-mcp.say"
description = "Echo text through MCP adapter"
effects = ["network", "dispatch_capability"]
default_permission = "ask"
parameters_schema = { type = "object", required = ["message"], properties = { message = { type = "string" } } }
"#;

const SCRIPT_MANIFEST: &str = r#"
id = "echo-script"
name = "Script Echo"
version = "0.1.0"
description = "Script echo demo extension"
trust = "sandbox"

[runtime]
kind = "script"
backend = "docker"
image = "alpine:latest"
command = "sh"
args = ["-c", "cat"]

[[capabilities]]
id = "echo-script.say"
description = "Echo text through Script Runner"
effects = ["dispatch_capability"]
default_permission = "allow"
parameters_schema = { type = "object", required = ["message"], properties = { message = { type = "string" } } }
"#;
