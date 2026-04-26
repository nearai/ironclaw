use std::{error::Error, sync::Arc};

use async_trait::async_trait;
use ironclaw_authorization::GrantAuthorizer;
use ironclaw_capabilities::{CapabilityHost, CapabilityInvocationRequest};
use ironclaw_dispatcher::RuntimeDispatcher;
use ironclaw_events::{JsonlEventSink, RuntimeEventKind};
use ironclaw_extensions::ExtensionDiscovery;
use ironclaw_filesystem::LocalFilesystem;
use ironclaw_host_api::{
    CapabilityGrant, CapabilityGrantId, CapabilityId, CapabilitySet, CorrelationId, EffectKind,
    ExecutionContext, ExtensionId, GrantConstraints, HostPath, InvocationId, MountView,
    NetworkPolicy, Principal, ProjectId, ResourceEstimate, ResourceScope, RuntimeKind, TenantId,
    TrustClass, UserId, VirtualPath,
};
use ironclaw_mcp::{McpClient, McpClientOutput, McpClientRequest, McpRuntime, McpRuntimeConfig};
use ironclaw_resources::InMemoryResourceGovernor;
use ironclaw_scripts::{
    DockerScriptBackend, ScriptBackend, ScriptBackendOutput, ScriptBackendRequest, ScriptRuntime,
    ScriptRuntimeConfig,
};
use ironclaw_wasm::WasmRuntime;
use serde_json::{Value, json};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    if std::env::var("IRONCLAW_REBORN_DEMO_DOCKER").as_deref() == Ok("1") {
        run_demo(DockerScriptBackend, "docker").await
    } else {
        run_demo(EchoScriptBackend, "in_process_echo").await
    }
}

async fn run_demo<B>(script_backend: B, script_backend_label: &str) -> Result<(), Box<dyn Error>>
where
    B: ScriptBackend + 'static,
{
    let fs = Arc::new(filesystem_with_echo_extensions()?);
    let registry =
        ExtensionDiscovery::discover(fs.as_ref(), &VirtualPath::new("/system/extensions")?).await?;
    let discovered_extensions = registry.extensions().count();

    let governor = InMemoryResourceGovernor::new();
    let wasm_runtime = WasmRuntime::for_testing()?;
    let script_runtime = ScriptRuntime::new(ScriptRuntimeConfig::for_testing(), script_backend);
    let mcp_runtime = McpRuntime::new(McpRuntimeConfig::for_testing(), EchoMcpClient);
    let authorizer = GrantAuthorizer::new();
    let context = sample_context()?;
    let event_path = VirtualPath::new("/engine/events/reborn-demo.jsonl")?;
    let events = JsonlEventSink::new(Arc::clone(&fs), event_path.clone());
    let dispatcher = RuntimeDispatcher::new(&registry, fs.as_ref(), &governor)
        .with_wasm_runtime(&wasm_runtime)
        .with_script_runtime(&script_runtime)
        .with_mcp_runtime(&mcp_runtime)
        .with_event_sink(&events);
    let host = CapabilityHost::new(&registry, &dispatcher, &authorizer);

    let wasm = host
        .invoke_json(CapabilityInvocationRequest {
            context: context.clone(),
            capability_id: CapabilityId::new("echo-wasm.say")?,
            estimate: ResourceEstimate {
                concurrency_slots: Some(1),
                output_bytes: Some(10_000),
                ..ResourceEstimate::default()
            },
            input: json!({"message": "hello wasm"}),
        })
        .await?
        .dispatch;

    let script = host
        .invoke_json(CapabilityInvocationRequest {
            context: context.clone(),
            capability_id: CapabilityId::new("echo-script.say")?,
            estimate: ResourceEstimate {
                concurrency_slots: Some(1),
                process_count: Some(1),
                output_bytes: Some(10_000),
                ..ResourceEstimate::default()
            },
            input: json!({"message": "hello script"}),
        })
        .await?
        .dispatch;

    let mcp = host
        .invoke_json(CapabilityInvocationRequest {
            context,
            capability_id: CapabilityId::new("echo-mcp.say")?,
            estimate: ResourceEstimate {
                concurrency_slots: Some(1),
                process_count: Some(1),
                output_bytes: Some(10_000),
                ..ResourceEstimate::default()
            },
            input: json!({"message": "hello mcp"}),
        })
        .await?
        .dispatch;

    let recorded_events = events.read_events().await?;

    println!("reborn_vertical_slice=ok");
    println!("discovered_extensions={discovered_extensions}");
    println!(
        "dispatch={} runtime={} output={} reservation_status={:?}",
        wasm.capability_id,
        runtime_label(wasm.runtime),
        stable_json(&wasm.output),
        wasm.receipt.status
    );
    println!(
        "dispatch={} runtime={} script_backend={} output={} reservation_status={:?}",
        script.capability_id,
        runtime_label(script.runtime),
        script_backend_label,
        stable_json(&script.output),
        script.receipt.status
    );
    println!(
        "dispatch={} runtime={} mcp_transport=stdio output={} reservation_status={:?}",
        mcp.capability_id,
        runtime_label(mcp.runtime),
        stable_json(&mcp.output),
        mcp.receipt.status
    );
    println!("durable_event_path={event_path:?}");
    println!("events={}", recorded_events.len());
    for (index, event) in recorded_events.iter().enumerate() {
        println!(
            "event[{index}]={} capability={} runtime={} error={}",
            event_kind_label(event.kind),
            event.capability_id,
            event.runtime.map(runtime_label).unwrap_or("none"),
            event.error_kind.as_deref().unwrap_or("none")
        );
    }
    Ok(())
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

fn filesystem_with_echo_extensions() -> Result<LocalFilesystem, Box<dyn Error>> {
    let storage = tempfile::tempdir()?.keep();
    let extensions_root = storage.join("extensions");
    let engine_root = storage.join("engine");
    std::fs::create_dir_all(&extensions_root)?;
    std::fs::create_dir_all(&engine_root)?;

    let wasm_root = extensions_root.join("echo-wasm");
    std::fs::create_dir_all(wasm_root.join("wasm"))?;
    std::fs::write(wasm_root.join("manifest.toml"), WASM_MANIFEST)?;
    std::fs::write(wasm_root.join("wasm/echo.wasm"), json_echo_module()?)?;

    let script_root = extensions_root.join("echo-script");
    std::fs::create_dir_all(&script_root)?;
    std::fs::write(script_root.join("manifest.toml"), SCRIPT_MANIFEST)?;

    let mcp_root = extensions_root.join("echo-mcp");
    std::fs::create_dir_all(&mcp_root)?;
    std::fs::write(mcp_root.join("manifest.toml"), MCP_MANIFEST)?;

    let mut fs = LocalFilesystem::new();
    fs.mount_local(
        VirtualPath::new("/system/extensions")?,
        HostPath::from_path_buf(extensions_root),
    )?;
    fs.mount_local(
        VirtualPath::new("/engine")?,
        HostPath::from_path_buf(engine_root),
    )?;
    Ok(fs)
}

fn json_echo_module() -> Result<Vec<u8>, wat::Error> {
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
}

fn sample_context() -> Result<ExecutionContext, Box<dyn Error>> {
    let invocation_id = InvocationId::new();
    let resource_scope = ResourceScope {
        tenant_id: TenantId::new("tenant1")?,
        user_id: UserId::new("user1")?,
        project_id: Some(ProjectId::new("project1")?),
        mission_id: None,
        thread_id: None,
        invocation_id,
    };
    let extension_id = ExtensionId::new("demo-host")?;
    Ok(ExecutionContext {
        invocation_id,
        correlation_id: CorrelationId::new(),
        process_id: None,
        parent_process_id: None,
        tenant_id: resource_scope.tenant_id.clone(),
        user_id: resource_scope.user_id.clone(),
        project_id: resource_scope.project_id.clone(),
        mission_id: resource_scope.mission_id.clone(),
        thread_id: resource_scope.thread_id.clone(),
        extension_id: extension_id.clone(),
        runtime: RuntimeKind::System,
        trust: TrustClass::System,
        grants: CapabilitySet {
            grants: vec![
                grant_for(
                    CapabilityId::new("echo-wasm.say")?,
                    extension_id.clone(),
                    vec![EffectKind::DispatchCapability],
                ),
                grant_for(
                    CapabilityId::new("echo-script.say")?,
                    extension_id.clone(),
                    vec![EffectKind::DispatchCapability],
                ),
                grant_for(
                    CapabilityId::new("echo-mcp.say")?,
                    extension_id,
                    vec![EffectKind::DispatchCapability, EffectKind::Network],
                ),
            ],
        },
        mounts: MountView::default(),
        resource_scope,
    })
}

fn grant_for(
    capability: CapabilityId,
    extension_id: ExtensionId,
    allowed_effects: Vec<EffectKind>,
) -> CapabilityGrant {
    CapabilityGrant {
        id: CapabilityGrantId::new(),
        capability,
        grantee: Principal::Extension(extension_id),
        issued_by: Principal::System,
        constraints: GrantConstraints {
            allowed_effects,
            mounts: MountView::default(),
            network: NetworkPolicy::default(),
            secrets: Vec::new(),
            resource_ceiling: None,
            expires_at: None,
            max_invocations: None,
        },
    }
}

fn event_kind_label(kind: RuntimeEventKind) -> &'static str {
    match kind {
        RuntimeEventKind::DispatchRequested => "dispatch_requested",
        RuntimeEventKind::RuntimeSelected => "runtime_selected",
        RuntimeEventKind::DispatchSucceeded => "dispatch_succeeded",
        RuntimeEventKind::DispatchFailed => "dispatch_failed",
        RuntimeEventKind::ProcessStarted => "process_started",
        RuntimeEventKind::ProcessCompleted => "process_completed",
        RuntimeEventKind::ProcessFailed => "process_failed",
        RuntimeEventKind::ProcessKilled => "process_killed",
    }
}

fn runtime_label(runtime: RuntimeKind) -> &'static str {
    match runtime {
        RuntimeKind::Wasm => "wasm",
        RuntimeKind::Script => "script",
        RuntimeKind::Mcp => "mcp",
        RuntimeKind::FirstParty => "first_party",
        RuntimeKind::System => "system",
    }
}

fn stable_json(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "null".to_string())
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
runner = "sandboxed_process"
command = "sh"
args = ["-c", "cat"]

[[capabilities]]
id = "echo-script.say"
description = "Echo text through Script Runner"
effects = ["dispatch_capability"]
default_permission = "allow"
parameters_schema = { type = "object", required = ["message"], properties = { message = { type = "string" } } }
"#;
