use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ironclaw_dispatcher::*;
use ironclaw_events::*;
use ironclaw_extensions::*;
use ironclaw_filesystem::*;
use ironclaw_host_api::*;
use ironclaw_mcp::*;
use ironclaw_resources::*;
use ironclaw_scripts::*;
use ironclaw_wasm::*;
use serde_json::json;

#[tokio::test]
async fn dispatcher_emits_events_for_wasm_and_script_success() {
    let fs = filesystem_with_echo_extensions();
    let registry =
        ExtensionDiscovery::discover(&fs, &VirtualPath::new("/system/extensions").unwrap())
            .await
            .unwrap();
    let governor = InMemoryResourceGovernor::new();
    let wasm_runtime = WasmRuntime::for_testing().unwrap();
    let script_runtime = ScriptRuntime::new(ScriptRuntimeConfig::for_testing(), EchoScriptBackend);
    let events = InMemoryEventSink::new();
    let dispatcher = RuntimeDispatcher::new(&registry, &fs, &governor)
        .with_wasm_runtime(&wasm_runtime)
        .with_script_runtime(&script_runtime)
        .with_event_sink(&events);

    dispatcher
        .dispatch_json(CapabilityDispatchRequest {
            capability_id: CapabilityId::new("echo-wasm.say").unwrap(),
            scope: sample_scope(),
            estimate: ResourceEstimate {
                concurrency_slots: Some(1),
                output_bytes: Some(10_000),
                ..ResourceEstimate::default()
            },
            input: json!({"message": "hello wasm"}),
        })
        .await
        .unwrap();

    dispatcher
        .dispatch_json(CapabilityDispatchRequest {
            capability_id: CapabilityId::new("echo-script.say").unwrap(),
            scope: sample_scope(),
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

    let recorded = events.events();
    let kinds = recorded.iter().map(|event| event.kind).collect::<Vec<_>>();
    assert_eq!(
        kinds,
        vec![
            RuntimeEventKind::DispatchRequested,
            RuntimeEventKind::RuntimeSelected,
            RuntimeEventKind::DispatchSucceeded,
            RuntimeEventKind::DispatchRequested,
            RuntimeEventKind::RuntimeSelected,
            RuntimeEventKind::DispatchSucceeded,
        ]
    );
    assert_eq!(
        recorded[0].capability_id,
        CapabilityId::new("echo-wasm.say").unwrap()
    );
    assert_eq!(recorded[1].runtime, Some(RuntimeKind::Wasm));
    assert_eq!(recorded[2].output_bytes, Some(24));
    assert_eq!(
        recorded[3].capability_id,
        CapabilityId::new("echo-script.say").unwrap()
    );
    assert_eq!(recorded[4].runtime, Some(RuntimeKind::Script));
    assert_eq!(
        recorded[5].provider,
        Some(ExtensionId::new("echo-script").unwrap())
    );
}

#[tokio::test]
async fn dispatcher_emits_events_for_mcp_success() {
    let fs = filesystem_with_echo_extensions();
    let mut registry = ExtensionRegistry::new();
    registry
        .insert(package_from_manifest(MCP_MANIFEST))
        .unwrap();
    let governor = InMemoryResourceGovernor::new();
    let mcp_runtime = McpRuntime::new(
        McpRuntimeConfig::for_testing(),
        RecordingMcpClient::new(McpClientOutput::json(json!({"matches": ["ironclaw"]}))),
    );
    let events = InMemoryEventSink::new();
    let dispatcher = RuntimeDispatcher::new(&registry, &fs, &governor)
        .with_mcp_runtime(&mcp_runtime)
        .with_event_sink(&events);

    dispatcher
        .dispatch_json(CapabilityDispatchRequest {
            capability_id: CapabilityId::new("github-mcp.search").unwrap(),
            scope: sample_scope(),
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

    let recorded = events.events();
    assert_eq!(recorded.len(), 3);
    assert_eq!(recorded[0].kind, RuntimeEventKind::DispatchRequested);
    assert_eq!(recorded[1].kind, RuntimeEventKind::RuntimeSelected);
    assert_eq!(recorded[1].runtime, Some(RuntimeKind::Mcp));
    assert_eq!(recorded[2].kind, RuntimeEventKind::DispatchSucceeded);
    assert_eq!(
        recorded[2].provider,
        Some(ExtensionId::new("github-mcp").unwrap())
    );
    assert!(recorded[2].output_bytes.unwrap() > 0);
}

#[tokio::test]
async fn dispatcher_event_sink_failures_do_not_fail_dispatch() {
    let fs = filesystem_with_echo_extensions();
    let registry =
        ExtensionDiscovery::discover(&fs, &VirtualPath::new("/system/extensions").unwrap())
            .await
            .unwrap();
    let governor = InMemoryResourceGovernor::new();
    let wasm_runtime = WasmRuntime::for_testing().unwrap();
    let dispatcher = RuntimeDispatcher::new(&registry, &fs, &governor)
        .with_wasm_runtime(&wasm_runtime)
        .with_event_sink(&FailingEventSink);
    let scope = sample_scope();
    let account = ResourceAccount::tenant(scope.tenant_id.clone());

    let result = dispatcher
        .dispatch_json(CapabilityDispatchRequest {
            capability_id: CapabilityId::new("echo-wasm.say").unwrap(),
            scope,
            estimate: ResourceEstimate {
                concurrency_slots: Some(1),
                output_bytes: Some(10_000),
                ..ResourceEstimate::default()
            },
            input: json!({"message": "event sink is down"}),
        })
        .await
        .unwrap();

    assert_eq!(result.output, json!({"message": "event sink is down"}));
    assert_eq!(governor.reserved_for(&account), ResourceTally::default());
    assert!(governor.usage_for(&account).output_bytes > 0);
}

#[tokio::test]
async fn dispatcher_can_persist_events_to_filesystem_jsonl_sink() {
    let (fs, event_path) = filesystem_with_echo_extensions_and_engine();
    let fs = Arc::new(fs);
    let registry = ExtensionDiscovery::discover(
        fs.as_ref(),
        &VirtualPath::new("/system/extensions").unwrap(),
    )
    .await
    .unwrap();
    let governor = InMemoryResourceGovernor::new();
    let wasm_runtime = WasmRuntime::for_testing().unwrap();
    let events = JsonlEventSink::new(Arc::clone(&fs), event_path.clone());
    let dispatcher = RuntimeDispatcher::new(&registry, fs.as_ref(), &governor)
        .with_wasm_runtime(&wasm_runtime)
        .with_event_sink(&events);

    dispatcher
        .dispatch_json(CapabilityDispatchRequest {
            capability_id: CapabilityId::new("echo-wasm.say").unwrap(),
            scope: sample_scope(),
            estimate: ResourceEstimate {
                concurrency_slots: Some(1),
                output_bytes: Some(10_000),
                ..ResourceEstimate::default()
            },
            input: json!({"message": "durable wasm"}),
        })
        .await
        .unwrap();

    let recorded = events.read_events().await.unwrap();
    assert_eq!(recorded.len(), 3);
    assert_eq!(recorded[0].kind, RuntimeEventKind::DispatchRequested);
    assert_eq!(recorded[1].kind, RuntimeEventKind::RuntimeSelected);
    assert_eq!(recorded[2].kind, RuntimeEventKind::DispatchSucceeded);

    let bytes = fs.read_file(&event_path).await.unwrap();
    let text = String::from_utf8(bytes).unwrap();
    assert_eq!(text.lines().count(), 3);
    assert!(!text.contains("/var/"));
    assert!(!text.contains("/tmp/"));
}

#[tokio::test]
async fn dispatcher_emits_failed_event_for_missing_backend_without_reserving() {
    let fs = filesystem_with_echo_extensions();
    let registry =
        ExtensionDiscovery::discover(&fs, &VirtualPath::new("/system/extensions").unwrap())
            .await
            .unwrap();
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope();
    let account = ResourceAccount::tenant(scope.tenant_id.clone());
    let events = InMemoryEventSink::new();
    let dispatcher = RuntimeDispatcher::new(&registry, &fs, &governor).with_event_sink(&events);

    let err = dispatcher
        .dispatch_json(CapabilityDispatchRequest {
            capability_id: CapabilityId::new("echo-script.say").unwrap(),
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

    assert_eq!(
        err.kind,
        CapabilityDispatchFailureKind::MissingRuntimeBackend
    );
    assert_eq!(err.runtime, Some(RuntimeKind::Script));
    assert_eq!(governor.reserved_for(&account), ResourceTally::default());
    assert_eq!(governor.usage_for(&account), ResourceTally::default());

    let recorded = events.events();
    assert_eq!(recorded.len(), 2);
    assert_eq!(recorded[0].kind, RuntimeEventKind::DispatchRequested);
    assert_eq!(recorded[1].kind, RuntimeEventKind::DispatchFailed);
    assert_eq!(recorded[1].runtime, Some(RuntimeKind::Script));
    assert_eq!(
        recorded[1].error_kind.as_ref().map(|kind| kind.as_str()),
        Some("MissingRuntimeBackend")
    );
}

#[tokio::test]
async fn event_sink_failure_does_not_change_dispatch_result() {
    let fs = filesystem_with_echo_extensions();
    let registry =
        ExtensionDiscovery::discover(&fs, &VirtualPath::new("/system/extensions").unwrap())
            .await
            .unwrap();
    let governor = InMemoryResourceGovernor::new();
    let wasm_runtime = WasmRuntime::for_testing().unwrap();
    let failing_events = FailingEventSink;
    let dispatcher = RuntimeDispatcher::new(&registry, &fs, &governor)
        .with_wasm_runtime(&wasm_runtime)
        .with_event_sink(&failing_events);

    let result = dispatcher
        .dispatch_json(CapabilityDispatchRequest {
            capability_id: CapabilityId::new("echo-wasm.say").unwrap(),
            scope: sample_scope(),
            estimate: ResourceEstimate {
                concurrency_slots: Some(1),
                output_bytes: Some(10_000),
                ..ResourceEstimate::default()
            },
            input: json!({"message": "hello wasm"}),
        })
        .await
        .unwrap();

    assert_eq!(result.output, json!({"message": "hello wasm"}));
    assert_eq!(result.receipt.status, ReservationStatus::Reconciled);
}

struct FailingEventSink;

#[async_trait]
impl EventSink for FailingEventSink {
    async fn emit(&self, _event: RuntimeEvent) -> Result<(), EventError> {
        Err(EventError::Sink {
            reason: "forced sink failure".to_string(),
        })
    }
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

fn filesystem_with_echo_extensions_and_engine() -> (LocalFilesystem, VirtualPath) {
    let storage = tempfile::tempdir().unwrap().keep();
    let extensions_root = storage.join("extensions");
    let engine_root = storage.join("engine");
    std::fs::create_dir_all(&extensions_root).unwrap();
    std::fs::create_dir_all(&engine_root).unwrap();

    write_echo_extensions(&extensions_root);

    let mut fs = LocalFilesystem::new();
    fs.mount_local(
        VirtualPath::new("/system/extensions").unwrap(),
        HostPath::from_path_buf(extensions_root),
    )
    .unwrap();
    fs.mount_local(
        VirtualPath::new("/engine").unwrap(),
        HostPath::from_path_buf(engine_root),
    )
    .unwrap();
    (
        fs,
        VirtualPath::new("/engine/events/reborn-test.jsonl").unwrap(),
    )
}

fn filesystem_with_echo_extensions() -> LocalFilesystem {
    let storage = tempfile::tempdir().unwrap().keep();
    write_echo_extensions(&storage);

    let mut fs = LocalFilesystem::new();
    fs.mount_local(
        VirtualPath::new("/system/extensions").unwrap(),
        HostPath::from_path_buf(storage),
    )
    .unwrap();
    fs
}

fn write_echo_extensions(root: &std::path::Path) {
    let wasm_root = root.join("echo-wasm");
    std::fs::create_dir_all(wasm_root.join("wasm")).unwrap();
    std::fs::write(wasm_root.join("manifest.toml"), WASM_MANIFEST).unwrap();
    std::fs::write(wasm_root.join("wasm/echo.wasm"), json_echo_module()).unwrap();

    let script_root = root.join("echo-script");
    std::fs::create_dir_all(&script_root).unwrap();
    std::fs::write(script_root.join("manifest.toml"), SCRIPT_MANIFEST).unwrap();
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

fn sample_scope() -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new("tenant-a").unwrap(),
        user_id: UserId::new("user-a").unwrap(),
        project_id: Some(ProjectId::new("project-a").unwrap()),
        mission_id: Some(MissionId::new("mission-a").unwrap()),
        thread_id: Some(ThreadId::new("thread-a").unwrap()),
        invocation_id: InvocationId::new(),
    }
}

const WASM_MANIFEST: &str = r#"
id = "echo-wasm"
name = "Echo WASM"
version = "0.1.0"
description = "Echo WASM demo extension"
trust = "sandbox"

[runtime]
kind = "wasm"
module = "wasm/echo.wasm"

[[capabilities]]
id = "echo-wasm.say"
description = "Echo WASM"
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
args = ["--stdio"]

[[capabilities]]
id = "github-mcp.search"
description = "Search GitHub"
effects = ["network", "dispatch_capability"]
default_permission = "ask"
parameters_schema = { type = "object" }
"#;

const SCRIPT_MANIFEST: &str = r#"
id = "echo-script"
name = "Echo Script"
version = "0.1.0"
description = "Echo Script demo extension"
trust = "sandbox"

[runtime]
kind = "script"
backend = "docker"
image = "alpine:latest"
command = "sh"
args = ["-c", "cat"]

[[capabilities]]
id = "echo-script.say"
description = "Echo script"
effects = ["dispatch_capability"]
default_permission = "allow"
parameters_schema = { type = "object" }
"#;
