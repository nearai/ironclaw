use std::{error::Error, sync::Arc};

use async_trait::async_trait;
use ironclaw_dispatcher::{
    DispatchError, RuntimeAdapter, RuntimeAdapterRequest, RuntimeAdapterResult, RuntimeDispatcher,
};
use ironclaw_events::{JsonlEventSink, RuntimeEventKind, scoped_runtime_event_log_path};
use ironclaw_extensions::ExtensionDiscovery;
use ironclaw_filesystem::LocalFilesystem;
use ironclaw_host_api::{
    CapabilityDispatchError, CapabilityDispatchFailureKind, CapabilityDispatchRequest,
    CapabilityId, HostPath, InvocationId, ProjectId, ResourceEstimate, ResourceScope,
    ResourceUsage, RuntimeKind, TenantId, UserId, VirtualPath,
};
use ironclaw_resources::{InMemoryResourceGovernor, ResourceGovernor};
use serde_json::{Value, json};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    let fs = Arc::new(filesystem_with_echo_extensions()?);
    let registry =
        ExtensionDiscovery::discover(fs.as_ref(), &VirtualPath::new("/system/extensions")?).await?;
    let discovered_extensions = registry.extensions().count();

    let governor = InMemoryResourceGovernor::new();
    let wasm_adapter = EchoAdapter::new(RuntimeKind::Wasm);
    let script_adapter = EchoAdapter::new(RuntimeKind::Script);
    let mcp_adapter = EchoAdapter::new(RuntimeKind::Mcp);
    let scope = sample_scope()?;
    let event_path = scoped_runtime_event_log_path(&scope, "reborn-demo.jsonl")?;
    let events = JsonlEventSink::new(Arc::clone(&fs), event_path.clone());
    let dispatcher = RuntimeDispatcher::new(&registry, fs.as_ref(), &governor)
        .with_runtime_adapter(RuntimeKind::Wasm, &wasm_adapter)
        .with_runtime_adapter(RuntimeKind::Script, &script_adapter)
        .with_runtime_adapter(RuntimeKind::Mcp, &mcp_adapter)
        .with_event_sink(&events);

    let wasm = dispatcher
        .dispatch_json(CapabilityDispatchRequest {
            capability_id: CapabilityId::new("echo-wasm.say")?,
            scope: scope.clone(),
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
            scope: scope.clone(),
            estimate: ResourceEstimate {
                concurrency_slots: Some(1),
                process_count: Some(1),
                output_bytes: Some(10_000),
                ..ResourceEstimate::default()
            },
            input: json!({"message": "hello script"}),
        })
        .await?;

    let mcp = dispatcher
        .dispatch_json(CapabilityDispatchRequest {
            capability_id: CapabilityId::new("echo-mcp.say")?,
            scope,
            estimate: ResourceEstimate {
                concurrency_slots: Some(1),
                process_count: Some(1),
                output_bytes: Some(10_000),
                ..ResourceEstimate::default()
            },
            input: json!({"message": "hello mcp"}),
        })
        .await?;

    let recorded_events = events.read_events().await?;

    println!("reborn_dispatcher_adapter_slice=ok");
    println!("discovered_extensions={discovered_extensions}");
    println!(
        "dispatch={} runtime={} output={} reservation_status={:?}",
        wasm.capability_id,
        runtime_label(wasm.runtime),
        stable_json(&wasm.output),
        wasm.receipt.status
    );
    println!(
        "dispatch={} runtime={} output={} reservation_status={:?}",
        script.capability_id,
        runtime_label(script.runtime),
        stable_json(&script.output),
        script.receipt.status
    );
    println!(
        "dispatch={} runtime={} output={} reservation_status={:?}",
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
            event
                .error_kind
                .as_ref()
                .map(|kind| kind.as_str())
                .unwrap_or("none")
        );
    }
    Ok(())
}

#[derive(Clone)]
struct EchoAdapter {
    runtime: RuntimeKind,
}

impl EchoAdapter {
    fn new(runtime: RuntimeKind) -> Self {
        Self { runtime }
    }
}

#[async_trait]
impl RuntimeAdapter<LocalFilesystem, InMemoryResourceGovernor> for EchoAdapter {
    async fn dispatch_json(
        &self,
        request: RuntimeAdapterRequest<'_, LocalFilesystem, InMemoryResourceGovernor>,
    ) -> Result<RuntimeAdapterResult, DispatchError> {
        let output = request.input;
        let usage = ResourceUsage {
            output_bytes: serde_json::to_vec(&output).unwrap().len() as u64,
            process_count: u32::from(matches!(
                self.runtime,
                RuntimeKind::Script | RuntimeKind::Mcp
            )),
            ..ResourceUsage::default()
        };
        let reservation = request
            .governor
            .reserve(request.scope, request.estimate)
            .map_err(|_| {
                dispatch_error_for_runtime(
                    request.capability_id,
                    &request.descriptor.provider,
                    self.runtime,
                )
            })?;
        let receipt = request
            .governor
            .reconcile(reservation.id, usage.clone())
            .map_err(|_| {
                dispatch_error_for_runtime(
                    request.capability_id,
                    &request.descriptor.provider,
                    self.runtime,
                )
            })?;
        Ok(RuntimeAdapterResult {
            output,
            output_bytes: usage.output_bytes,
            usage,
            receipt,
        })
    }
}

fn dispatch_error_for_runtime(
    capability_id: &CapabilityId,
    provider: &ironclaw_host_api::ExtensionId,
    runtime: RuntimeKind,
) -> CapabilityDispatchError {
    let kind = match runtime {
        RuntimeKind::Wasm => CapabilityDispatchFailureKind::Wasm,
        RuntimeKind::Script => CapabilityDispatchFailureKind::Script,
        RuntimeKind::Mcp => CapabilityDispatchFailureKind::Mcp,
        RuntimeKind::FirstParty | RuntimeKind::System => {
            CapabilityDispatchFailureKind::UnsupportedRuntime
        }
    };
    CapabilityDispatchError::new(
        kind,
        capability_id.clone(),
        Some(provider.clone()),
        Some(runtime),
    )
}

fn filesystem_with_echo_extensions() -> Result<LocalFilesystem, Box<dyn Error>> {
    let storage = tempfile::tempdir()?.keep();
    let extensions_root = storage.join("extensions");
    let engine_root = storage.join("engine");
    std::fs::create_dir_all(&extensions_root)?;
    std::fs::create_dir_all(&engine_root)?;

    let wasm_root = extensions_root.join("echo-wasm");
    std::fs::create_dir_all(&wasm_root)?;
    std::fs::write(wasm_root.join("manifest.toml"), WASM_MANIFEST)?;

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
