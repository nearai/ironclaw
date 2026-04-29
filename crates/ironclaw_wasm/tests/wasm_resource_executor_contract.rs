use std::sync::{Arc, Mutex};

use ironclaw_extensions::*;
use ironclaw_filesystem::*;
use ironclaw_host_api::*;
use ironclaw_resources::*;
use ironclaw_wasm::*;
use serde_json::json;

#[tokio::test]
async fn executor_reserves_invokes_and_reconciles_success() {
    let (fs, package) = wasm_package_with_module(json_echo_module()).await;
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

    let capability_id = CapabilityId::new("echo.say").unwrap();
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
                    output_bytes: Some(10_000),
                    ..ResourceEstimate::default()
                },
                resource_reservation: None,
                invocation: CapabilityInvocation {
                    input: json!({"message": "hello executor"}),
                },
            },
        )
        .await
        .unwrap();

    assert_eq!(
        execution.result.output,
        json!({"message": "hello executor"})
    );
    assert_eq!(execution.receipt.status, ReservationStatus::Reconciled);
    assert_eq!(governor.reserved_for(&account), ResourceTally::default());
    assert!(governor.usage_for(&account).output_bytes > 0);
    assert_eq!(governor.usage_for(&account).concurrency_slots, 0);
}

#[tokio::test]
async fn executor_returns_resource_error_without_reserving_when_budget_denied() {
    let (fs, package) = wasm_package_with_module(json_echo_module()).await;
    let runtime = WasmRuntime::for_testing().unwrap();
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope();
    let account = ResourceAccount::tenant(scope.tenant_id.clone());
    governor.set_limit(
        account.clone(),
        ResourceLimits {
            max_output_bytes: Some(1),
            ..ResourceLimits::default()
        },
    );

    let capability_id = CapabilityId::new("echo.say").unwrap();
    let err = runtime
        .execute_extension_json(
            &fs,
            &governor,
            WasmExecutionRequest {
                package: &package,
                capability_id: &capability_id,
                scope,
                estimate: ResourceEstimate {
                    output_bytes: Some(10_000),
                    ..ResourceEstimate::default()
                },
                resource_reservation: None,
                invocation: CapabilityInvocation {
                    input: json!({"message": "hello executor"}),
                },
            },
        )
        .await
        .unwrap_err();

    assert!(matches!(err, WasmError::Resource(_)));
    assert_eq!(governor.reserved_for(&account), ResourceTally::default());
    assert_eq!(governor.usage_for(&account), ResourceTally::default());
}

#[tokio::test]
async fn executor_releases_reservation_when_module_prepare_fails() {
    let (fs, package) = wasm_package_without_module().await;
    let runtime = WasmRuntime::for_testing().unwrap();
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope();
    let account = ResourceAccount::tenant(scope.tenant_id.clone());
    governor.set_limit(
        account.clone(),
        ResourceLimits {
            max_concurrency_slots: Some(1),
            ..ResourceLimits::default()
        },
    );

    let capability_id = CapabilityId::new("echo.say").unwrap();
    let err = runtime
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
                    input: json!({"message": "hello executor"}),
                },
            },
        )
        .await
        .unwrap_err();

    assert!(matches!(err, WasmError::Filesystem(_)));
    assert_eq!(governor.reserved_for(&account), ResourceTally::default());
    assert_eq!(governor.usage_for(&account), ResourceTally::default());
}

#[tokio::test]
async fn executor_releases_reservation_when_guest_traps() {
    let (fs, package) = wasm_package_with_module(trapping_module()).await;
    let runtime = WasmRuntime::for_testing().unwrap();
    let governor = InMemoryResourceGovernor::new();
    let scope = sample_scope();
    let account = ResourceAccount::tenant(scope.tenant_id.clone());
    governor.set_limit(
        account.clone(),
        ResourceLimits {
            max_concurrency_slots: Some(1),
            ..ResourceLimits::default()
        },
    );

    let capability_id = CapabilityId::new("echo.say").unwrap();
    let err = runtime
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
                    input: json!({"message": "hello executor"}),
                },
            },
        )
        .await
        .unwrap_err();

    assert!(matches!(err, WasmError::Trap { .. }));
    assert_eq!(governor.reserved_for(&account), ResourceTally::default());
    assert_eq!(governor.usage_for(&account), ResourceTally::default());
}

#[tokio::test]
async fn executor_releases_reservation_when_output_limit_fails() {
    let (fs, package) = wasm_package_with_module(json_echo_module()).await;
    let runtime = WasmRuntime::new(WasmRuntimeConfig {
        max_output_bytes: 4,
        ..WasmRuntimeConfig::for_testing()
    })
    .unwrap();
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

    let capability_id = CapabilityId::new("echo.say").unwrap();
    let err = runtime
        .execute_extension_json(
            &fs,
            &governor,
            WasmExecutionRequest {
                package: &package,
                capability_id: &capability_id,
                scope,
                estimate: ResourceEstimate {
                    concurrency_slots: Some(1),
                    output_bytes: Some(10_000),
                    ..ResourceEstimate::default()
                },
                resource_reservation: None,
                invocation: CapabilityInvocation {
                    input: json!({"message": "hello executor"}),
                },
            },
        )
        .await
        .unwrap_err();

    assert!(matches!(err, WasmError::OutputLimitExceeded { .. }));
    assert_eq!(governor.reserved_for(&account), ResourceTally::default());
    assert_eq!(governor.usage_for(&account), ResourceTally::default());
}

#[tokio::test]
async fn executor_host_context_supports_filesystem_and_network_imports_together() {
    let (fs, package) = wasm_package_with_module(fs_and_http_module()).await;
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
    let filesystem = StaticFilesystem::default();
    let http = RecordingHttpClient::new(WasmHttpResponse {
        status: 200,
        body: r#"{"ok":true}"#.to_string(),
    });
    let capability_id = CapabilityId::new("echo.say").unwrap();

    let execution = runtime
        .execute_extension_json_with_host_context(
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
            WasmHostImportContext::new()
                .with_filesystem(Arc::new(filesystem.clone()))
                .with_http(Arc::new(http.clone())),
        )
        .await
        .unwrap();

    assert_eq!(execution.result.output, json!({"ok": true}));
    assert_eq!(
        filesystem.read_paths.lock().unwrap().as_slice(),
        ["/workspace/input.json"]
    );
    assert_eq!(http.requests.lock().unwrap().len(), 1);
}

async fn wasm_package_with_module(bytes: Vec<u8>) -> (LocalFilesystem, ExtensionPackage) {
    let storage = tempfile::tempdir().unwrap().keep();
    std::fs::create_dir_all(storage.join("echo/wasm")).unwrap();
    std::fs::write(storage.join("echo/wasm/echo.wasm"), bytes).unwrap();

    let mut fs = LocalFilesystem::new();
    fs.mount_local(
        VirtualPath::new("/system/extensions").unwrap(),
        HostPath::from_path_buf(storage),
    )
    .unwrap();
    (fs, echo_package())
}

async fn wasm_package_without_module() -> (LocalFilesystem, ExtensionPackage) {
    let storage = tempfile::tempdir().unwrap().keep();
    std::fs::create_dir_all(storage.join("echo/wasm")).unwrap();

    let mut fs = LocalFilesystem::new();
    fs.mount_local(
        VirtualPath::new("/system/extensions").unwrap(),
        HostPath::from_path_buf(storage),
    )
    .unwrap();
    (fs, echo_package())
}

fn echo_package() -> ExtensionPackage {
    ExtensionPackage::from_manifest(
        ExtensionManifest::parse(WASM_MANIFEST).unwrap(),
        VirtualPath::new("/system/extensions/echo").unwrap(),
    )
    .unwrap()
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

fn fs_and_http_module() -> Vec<u8> {
    wat::parse_str(
        r#"(module
            (import "host" "fs_read_utf8" (func $read (param i32 i32 i32 i32) (result i32)))
            (import "host" "http_request_utf8" (func $http (param i32 i32 i32 i32 i32 i32 i32) (result i32)))
            (memory (export "memory") 1)
            (data (i32.const 64) "/workspace/input.json")
            (data (i32.const 128) "https://api.example.test/v1/echo")
            (data (i32.const 256) "{\"ok\":false}")
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
            (func (export "say") (param i32 i32) (result i32)
              (local $n i32)
              i32.const 64
              i32.const 21
              i32.const 2048
              i32.const 512
              call $read
              local.set $n
              local.get $n
              i32.const 0
              i32.lt_s
              if
                i32.const 256
                global.set $out_ptr
                i32.const 12
                global.set $out_len
                i32.const 0
                return
              end
              i32.const 0
              i32.const 128
              i32.const 32
              i32.const 0
              i32.const 0
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
            (func (export "output_ptr") (result i32)
              global.get $out_ptr)
            (func (export "output_len") (result i32)
              global.get $out_len))"#,
    )
    .unwrap()
}

fn trapping_module() -> Vec<u8> {
    wat::parse_str(
        r#"(module
            (memory (export "memory") 1)
            (global $heap (mut i32) (i32.const 1024))
            (func (export "alloc") (param $len i32) (result i32)
              global.get $heap)
            (func (export "say") (param i32 i32) (result i32)
              unreachable)
            (func (export "output_ptr") (result i32)
              i32.const 0)
            (func (export "output_len") (result i32)
              i32.const 0))"#,
    )
    .unwrap()
}

#[derive(Clone, Default)]
struct StaticFilesystem {
    read_paths: Arc<Mutex<Vec<String>>>,
}

impl WasmHostFilesystem for StaticFilesystem {
    fn read_utf8(&self, path: &str) -> Result<String, String> {
        self.read_paths.lock().unwrap().push(path.to_string());
        Ok(r#"{"fs":true}"#.to_string())
    }

    fn write_utf8(&self, _path: &str, _contents: &str) -> Result<(), String> {
        Ok(())
    }

    fn list_utf8(&self, _path: &str) -> Result<String, String> {
        Ok("[]".to_string())
    }

    fn stat_len(&self, _path: &str) -> Result<u64, String> {
        Ok(0)
    }
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
    fn request_utf8(
        &self,
        request: WasmHttpRequest,
    ) -> Result<WasmHttpResponse, WasmHostHttpError> {
        self.requests.lock().unwrap().push(request);
        Ok(self.response.clone())
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

const WASM_MANIFEST: &str = r#"
id = "echo"
name = "Echo"
version = "0.1.0"
description = "Echo demo extension"
trust = "untrusted"

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
