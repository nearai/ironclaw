use ironclaw_events::*;
use ironclaw_extensions::*;
use ironclaw_filesystem::*;
use ironclaw_host_api::*;
use ironclaw_kernel::*;
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

    assert!(matches!(
        err,
        DispatchError::MissingRuntimeBackend {
            runtime: RuntimeKind::Script
        }
    ));
    assert_eq!(governor.reserved_for(&account), ResourceTally::default());
    assert_eq!(governor.usage_for(&account), ResourceTally::default());

    let recorded = events.events();
    assert_eq!(recorded.len(), 2);
    assert_eq!(recorded[0].kind, RuntimeEventKind::DispatchRequested);
    assert_eq!(recorded[1].kind, RuntimeEventKind::DispatchFailed);
    assert_eq!(recorded[1].runtime, Some(RuntimeKind::Script));
    assert_eq!(
        recorded[1].error_kind.as_deref(),
        Some("MissingRuntimeBackend")
    );
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
