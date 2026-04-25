use std::error::Error;

use ironclaw_extensions::ExtensionDiscovery;
use ironclaw_filesystem::LocalFilesystem;
use ironclaw_host_api::{
    CapabilityId, HostPath, InvocationId, ProjectId, ResourceEstimate, ResourceScope, RuntimeKind,
    TenantId, UserId, VirtualPath,
};
use ironclaw_kernel::{CapabilityDispatchRequest, RuntimeDispatcher};
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
    let fs = filesystem_with_echo_extensions()?;
    let registry =
        ExtensionDiscovery::discover(&fs, &VirtualPath::new("/system/extensions")?).await?;
    let discovered_extensions = registry.extensions().count();

    let governor = InMemoryResourceGovernor::new();
    let wasm_runtime = WasmRuntime::for_testing()?;
    let script_runtime = ScriptRuntime::new(ScriptRuntimeConfig::for_testing(), script_backend);
    let dispatcher = RuntimeDispatcher::new(&registry, &fs, &governor)
        .with_wasm_runtime(&wasm_runtime)
        .with_script_runtime(&script_runtime);

    let wasm = dispatcher
        .dispatch_json(CapabilityDispatchRequest {
            capability_id: CapabilityId::new("echo-wasm.say")?,
            scope: sample_scope()?,
            estimate: ResourceEstimate {
                concurrency_slots: Some(1),
                output_bytes: Some(10_000),
                ..ResourceEstimate::default()
            },
            input: json!({"message": "hello wasm"}),
        })
        .await?;

    let script = dispatcher
        .dispatch_json(CapabilityDispatchRequest {
            capability_id: CapabilityId::new("echo-script.say")?,
            scope: sample_scope()?,
            estimate: ResourceEstimate {
                concurrency_slots: Some(1),
                process_count: Some(1),
                output_bytes: Some(10_000),
                ..ResourceEstimate::default()
            },
            input: json!({"message": "hello script"}),
        })
        .await?;

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
    Ok(())
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
    let wasm_root = storage.join("echo-wasm");
    std::fs::create_dir_all(wasm_root.join("wasm"))?;
    std::fs::write(wasm_root.join("manifest.toml"), WASM_MANIFEST)?;
    std::fs::write(wasm_root.join("wasm/echo.wasm"), json_echo_module()?)?;

    let script_root = storage.join("echo-script");
    std::fs::create_dir_all(&script_root)?;
    std::fs::write(script_root.join("manifest.toml"), SCRIPT_MANIFEST)?;

    let mut fs = LocalFilesystem::new();
    fs.mount_local(
        VirtualPath::new("/system/extensions")?,
        HostPath::from_path_buf(storage),
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

fn sample_scope() -> Result<ResourceScope, Box<dyn Error>> {
    Ok(ResourceScope {
        tenant_id: TenantId::new("tenant1")?,
        user_id: UserId::new("user1")?,
        project_id: Some(ProjectId::new("project1")?),
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    })
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
