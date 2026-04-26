use std::sync::Arc;

use ironclaw_extensions::*;
use ironclaw_filesystem::*;
use ironclaw_host_api::*;
use ironclaw_wasm::*;
use serde_json::json;
use tempfile::tempdir;

#[tokio::test]
async fn manifest_loader_reads_extension_module_and_invokes_json_capability() {
    let (fs, package) = wasm_package_with_module(json_echo_module()).await;
    let runtime = WasmRuntime::new(WasmRuntimeConfig {
        cache_compiled_modules: true,
        ..WasmRuntimeConfig::for_testing()
    })
    .unwrap();

    let prepared = runtime
        .prepare_extension_capability(&fs, &package, &CapabilityId::new("echo.say").unwrap())
        .await
        .unwrap();

    assert_eq!(prepared.descriptor.id.as_str(), "echo.say");
    assert_eq!(prepared.descriptor.runtime, RuntimeKind::Wasm);
    assert_eq!(prepared.module.provider().as_str(), "echo");
    assert_eq!(prepared.module.capability().as_str(), "echo.say");
    assert_eq!(prepared.module.export(), "say");
    assert_eq!(
        prepared.module_path,
        VirtualPath::new("/system/extensions/echo/wasm/echo.wasm").unwrap()
    );

    let result = runtime
        .invoke_json(
            prepared.module.as_ref(),
            &prepared.descriptor,
            Some(&sample_reservation()),
            CapabilityInvocation {
                input: json!({"message": "hello from manifest"}),
            },
        )
        .unwrap();

    assert_eq!(result.output, json!({"message": "hello from manifest"}));
}

#[tokio::test]
async fn manifest_loader_uses_runtime_cache_for_same_package_asset() {
    let (fs, package) = wasm_package_with_module(json_echo_module()).await;
    let runtime = WasmRuntime::new(WasmRuntimeConfig {
        cache_compiled_modules: true,
        ..WasmRuntimeConfig::for_testing()
    })
    .unwrap();

    let first = runtime
        .prepare_extension_capability(&fs, &package, &CapabilityId::new("echo.say").unwrap())
        .await
        .unwrap();
    let second = runtime
        .prepare_extension_capability(&fs, &package, &CapabilityId::new("echo.say").unwrap())
        .await
        .unwrap();

    assert!(Arc::ptr_eq(&first.module, &second.module));
    assert_eq!(runtime.prepared_module_count(), 1);
}

#[tokio::test]
async fn manifest_loader_rejects_non_wasm_runtime() {
    let storage = tempdir().unwrap();
    std::fs::create_dir_all(storage.path().join("script-tools")).unwrap();
    let package = package_from_manifest(
        SCRIPT_MANIFEST,
        VirtualPath::new("/system/extensions/script-tools").unwrap(),
    );
    let mut fs = LocalFilesystem::new();
    fs.mount_local(
        VirtualPath::new("/system/extensions").unwrap(),
        HostPath::from_path_buf(storage.path().to_path_buf()),
    )
    .unwrap();
    let runtime = WasmRuntime::for_testing().unwrap();

    let err = runtime
        .prepare_extension_capability(
            &fs,
            &package,
            &CapabilityId::new("script-tools.run").unwrap(),
        )
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        WasmError::ExtensionRuntimeMismatch { extension, actual }
            if extension.as_str() == "script-tools" && actual == RuntimeKind::Script
    ));
}

#[tokio::test]
async fn manifest_loader_rejects_undeclared_capability() {
    let (fs, package) = wasm_package_with_module(json_echo_module()).await;
    let runtime = WasmRuntime::for_testing().unwrap();

    let err = runtime
        .prepare_extension_capability(&fs, &package, &CapabilityId::new("echo.missing").unwrap())
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        WasmError::CapabilityNotDeclared { capability } if capability.as_str() == "echo.missing"
    ));
}

#[tokio::test]
async fn manifest_loader_rejects_capability_export_mismatch() {
    let (fs, package) = wasm_package_with_module(module_exporting_other_name()).await;
    let runtime = WasmRuntime::for_testing().unwrap();

    let err = runtime
        .prepare_extension_capability(&fs, &package, &CapabilityId::new("echo.say").unwrap())
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        WasmError::MissingExport { export } if export == "say"
    ));
}

#[tokio::test]
async fn manifest_loader_reads_only_manifest_declared_asset_under_package_root() {
    let storage = tempdir().unwrap();
    std::fs::create_dir_all(storage.path().join("echo/wasm")).unwrap();
    std::fs::create_dir_all(storage.path().join("other/wasm")).unwrap();
    std::fs::write(
        storage.path().join("other/wasm/echo.wasm"),
        json_echo_module(),
    )
    .unwrap();
    let package = package_from_manifest(
        WASM_MANIFEST,
        VirtualPath::new("/system/extensions/echo").unwrap(),
    );
    let mut fs = LocalFilesystem::new();
    fs.mount_local(
        VirtualPath::new("/system/extensions").unwrap(),
        HostPath::from_path_buf(storage.path().to_path_buf()),
    )
    .unwrap();
    let runtime = WasmRuntime::for_testing().unwrap();

    let err = runtime
        .prepare_extension_capability(&fs, &package, &CapabilityId::new("echo.say").unwrap())
        .await
        .unwrap_err();

    assert!(matches!(err, WasmError::Filesystem(_)));
    assert!(
        !err.to_string()
            .contains(&storage.path().display().to_string())
    );
}

async fn wasm_package_with_module(bytes: Vec<u8>) -> (LocalFilesystem, ExtensionPackage) {
    let storage = tempdir().unwrap().keep();
    std::fs::create_dir_all(storage.join("echo/wasm")).unwrap();
    std::fs::write(storage.join("echo/wasm/echo.wasm"), bytes).unwrap();

    let mut fs = LocalFilesystem::new();
    fs.mount_local(
        VirtualPath::new("/system/extensions").unwrap(),
        HostPath::from_path_buf(storage),
    )
    .unwrap();

    let package = package_from_manifest(
        WASM_MANIFEST,
        VirtualPath::new("/system/extensions/echo").unwrap(),
    );
    (fs, package)
}

fn package_from_manifest(input: &str, root: VirtualPath) -> ExtensionPackage {
    ExtensionPackage::from_manifest(ExtensionManifest::parse(input).unwrap(), root).unwrap()
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

fn module_exporting_other_name() -> Vec<u8> {
    wat::parse_str(
        r#"(module
            (memory (export "memory") 1)
            (func (export "other") (param i32 i32) (result i32) i32.const 0)
            (func (export "alloc") (param i32) (result i32) i32.const 0)
            (func (export "output_ptr") (result i32) i32.const 0)
            (func (export "output_len") (result i32) i32.const 2)
            (data (i32.const 0) "{}"))"#,
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

fn sample_reservation() -> ResourceReservation {
    ResourceReservation {
        id: ResourceReservationId::new(),
        scope: sample_scope(),
        estimate: ResourceEstimate::default(),
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
id = "script-tools"
name = "Script Tools"
version = "0.1.0"
description = "Script demo extension"
trust = "sandbox"

[runtime]
kind = "script"
runner = "sandboxed_process"
command = "run"
args = []

[[capabilities]]
id = "script-tools.run"
description = "Run script"
effects = ["execute_code"]
default_permission = "allow"
parameters_schema = { type = "object" }
"#;
