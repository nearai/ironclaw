use ironclaw_extensions::*;
use ironclaw_filesystem::*;
use ironclaw_host_api::*;
use ironclaw_resources::*;
use ironclaw_wasm::*;
use serde_json::json;

#[test]
fn core_log_import_records_guest_log_without_privileged_authority() {
    let runtime = WasmRuntime::for_testing().unwrap();
    let module = runtime.prepare(log_module_spec()).unwrap();
    let descriptor = make_descriptor("host-core", "host-core.log", RuntimeKind::Wasm);
    let reservation = sample_reservation();

    let result = runtime
        .invoke_json(
            &module,
            &descriptor,
            Some(&reservation),
            CapabilityInvocation {
                input: json!({"message": "hello"}),
            },
        )
        .unwrap();

    assert_eq!(result.output, json!({"message": "hello"}));
    assert_eq!(result.logs.len(), 1);
    assert_eq!(result.logs[0].level, WasmLogLevel::Info);
    assert_eq!(result.logs[0].message, "hello host log");
    assert!(result.logs[0].timestamp_unix_ms > 0);
}

#[test]
fn core_time_import_is_available_to_guest() {
    let runtime = WasmRuntime::for_testing().unwrap();
    let module = runtime.prepare(time_module_spec()).unwrap();
    let descriptor = make_descriptor("host-core", "host-core.time", RuntimeKind::Wasm);
    let reservation = sample_reservation();

    let result = runtime
        .invoke_json(
            &module,
            &descriptor,
            Some(&reservation),
            CapabilityInvocation { input: json!({}) },
        )
        .unwrap();

    assert_eq!(result.output, json!({"ok": true}));
    assert!(result.logs.is_empty());
}

#[test]
fn unsupported_host_imports_still_fail_closed() {
    let runtime = WasmRuntime::for_testing().unwrap();
    let err = runtime
        .prepare(WasmModuleSpec {
            provider: ExtensionId::new("host-core").unwrap(),
            capability: CapabilityId::new("host-core.bad").unwrap(),
            export: "run".to_string(),
            bytes: wat::parse_str(
                r#"(module
                    (import "host" "http_request" (func $http (param i32 i32) (result i32)))
                    (func (export "run") (param i32 i32) (result i32)
                      i32.const 0))"#,
            )
            .unwrap(),
        })
        .unwrap_err();

    assert!(matches!(
        err,
        WasmError::UnsupportedImport { module, name }
            if module == "host" && name == "http_request"
    ));
}

#[tokio::test]
async fn resource_executor_returns_logs_from_core_imports() {
    let (fs, package) = wasm_package_with_module(log_module_bytes()).await;
    let runtime = WasmRuntime::for_testing().unwrap();
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope();
    governor.set_limit(
        ResourceAccount::tenant(scope.tenant_id.clone()),
        ResourceLimits {
            max_concurrency_slots: Some(1),
            ..ResourceLimits::default()
        },
    );
    let capability_id = CapabilityId::new("host-core.log").unwrap();

    let execution = runtime
        .execute_extension_json(
            &fs,
            &governor,
            WasmExecutionRequest {
                package: &package,
                capability_id: &capability_id,
                scope,
                estimate: ResourceEstimate {
                    concurrency_slots: Some(1),
                    ..ResourceEstimate::default()
                },
                resource_reservation: None,
                invocation: CapabilityInvocation {
                    input: json!({"message": "hello"}),
                },
            },
        )
        .await
        .unwrap();

    assert_eq!(execution.result.logs.len(), 1);
    assert_eq!(execution.result.logs[0].message, "hello host log");
    assert_eq!(execution.receipt.status, ReservationStatus::Reconciled);
}

fn log_module_spec() -> WasmModuleSpec {
    WasmModuleSpec {
        provider: ExtensionId::new("host-core").unwrap(),
        capability: CapabilityId::new("host-core.log").unwrap(),
        export: "log".to_string(),
        bytes: log_module_bytes(),
    }
}

fn time_module_spec() -> WasmModuleSpec {
    WasmModuleSpec {
        provider: ExtensionId::new("host-core").unwrap(),
        capability: CapabilityId::new("host-core.time").unwrap(),
        export: "time".to_string(),
        bytes: time_module_bytes(),
    }
}

fn log_module_bytes() -> Vec<u8> {
    wat::parse_str(
        r#"(module
            (import "host" "log_utf8" (func $log (param i32 i32 i32) (result i32)))
            (memory (export "memory") 1)
            (data (i32.const 64) "hello host log")
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
            (func (export "log") (param $ptr i32) (param $len i32) (result i32)
              i32.const 2
              i32.const 64
              i32.const 14
              call $log
              drop
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

fn time_module_bytes() -> Vec<u8> {
    wat::parse_str(
        r#"(module
            (import "host" "time_unix_ms" (func $time (result i64)))
            (memory (export "memory") 1)
            (data (i32.const 64) "{\"ok\":true}")
            (data (i32.const 80) "{\"ok\":false}")
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
            (func (export "time") (param i32 i32) (result i32)
              call $time
              i64.const 0
              i64.gt_u
              if
                i32.const 64
                global.set $out_ptr
                i32.const 11
                global.set $out_len
              else
                i32.const 80
                global.set $out_ptr
                i32.const 12
                global.set $out_len
              end
              i32.const 0)
            (func (export "output_ptr") (result i32)
              global.get $out_ptr)
            (func (export "output_len") (result i32)
              global.get $out_len))"#,
    )
    .unwrap()
}

async fn wasm_package_with_module(bytes: Vec<u8>) -> (LocalFilesystem, ExtensionPackage) {
    let storage = tempfile::tempdir().unwrap().keep();
    std::fs::create_dir_all(storage.join("host-core/wasm")).unwrap();
    std::fs::write(storage.join("host-core/wasm/core.wasm"), bytes).unwrap();

    let mut fs = LocalFilesystem::new();
    fs.mount_local(
        VirtualPath::new("/system/extensions").unwrap(),
        HostPath::from_path_buf(storage),
    )
    .unwrap();

    let package = ExtensionPackage::from_manifest(
        ExtensionManifest::parse(WASM_MANIFEST).unwrap(),
        VirtualPath::new("/system/extensions/host-core").unwrap(),
    )
    .unwrap();
    (fs, package)
}

fn make_descriptor(provider: &str, capability: &str, runtime: RuntimeKind) -> CapabilityDescriptor {
    CapabilityDescriptor {
        id: CapabilityId::new(capability).unwrap(),
        provider: ExtensionId::new(provider).unwrap(),
        runtime,
        trust_ceiling: TrustClass::Sandbox,
        description: "test capability".to_string(),
        parameters_schema: serde_json::json!({"type":"object"}),
        effects: vec![EffectKind::DispatchCapability],
        default_permission: PermissionMode::Allow,
        resource_profile: None,
    }
}

fn sample_scope() -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new("tenant1").unwrap(),
        user_id: UserId::new("user1").unwrap(),
        agent_id: None,
        project_id: Some(ProjectId::new("project1").unwrap()),
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    }
}

fn sample_reservation() -> ResourceReservation {
    ResourceReservation {
        id: ResourceReservationId::new(),
        scope: sample_scope(),
        estimate: ResourceEstimate::default(),
    }
}

const WASM_MANIFEST: &str = r#"
id = "host-core"
name = "Host Core"
version = "0.1.0"
description = "Host core demo extension"
trust = "sandbox"

[runtime]
kind = "wasm"
module = "wasm/core.wasm"

[[capabilities]]
id = "host-core.log"
description = "Log text"
effects = ["dispatch_capability"]
default_permission = "allow"
parameters_schema = { type = "object" }
"#;
