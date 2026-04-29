use std::sync::{Arc, Mutex};
use std::thread::ThreadId;

use async_trait::async_trait;

use ironclaw_filesystem::*;
use ironclaw_host_api::*;
use ironclaw_wasm::*;
use serde_json::json;
use tempfile::tempdir;

#[test]
fn fs_read_import_uses_scoped_filesystem_mounts() {
    let (wasm_fs, _storage) = scoped_filesystem(MountPermissions::read_only(), |project| {
        std::fs::write(project.join("input.json"), br#"{"text":"from fs"}"#).unwrap();
    });
    let runtime = WasmRuntime::for_testing().unwrap();
    let module = runtime.prepare(fs_read_spec()).unwrap();
    let descriptor = make_descriptor("fs-demo", "fs-demo.read", RuntimeKind::Wasm);
    let reservation = sample_reservation();

    let result = runtime
        .invoke_json_with_filesystem(
            &module,
            &descriptor,
            Some(&reservation),
            CapabilityInvocation { input: json!({}) },
            Arc::new(wasm_fs),
        )
        .unwrap();

    assert_eq!(result.output, json!({"text": "from fs"}));
}

#[test]
fn fs_imports_deny_by_default_without_filesystem_context() {
    let runtime = WasmRuntime::for_testing().unwrap();
    let module = runtime.prepare(fs_read_spec()).unwrap();
    let descriptor = make_descriptor("fs-demo", "fs-demo.read", RuntimeKind::Wasm);
    let reservation = sample_reservation();

    let result = runtime
        .invoke_json(
            &module,
            &descriptor,
            Some(&reservation),
            CapabilityInvocation { input: json!({}) },
        )
        .unwrap();

    assert_eq!(result.output, json!({"ok": false}));
}

#[test]
fn fs_write_import_respects_mount_permissions() {
    let (read_write_fs, storage) = scoped_filesystem(MountPermissions::read_write(), |_| {});
    let runtime = WasmRuntime::for_testing().unwrap();
    let module = runtime.prepare(fs_write_spec()).unwrap();
    let descriptor = make_descriptor("fs-demo", "fs-demo.write", RuntimeKind::Wasm);
    let reservation = sample_reservation();

    let result = runtime
        .invoke_json_with_filesystem(
            &module,
            &descriptor,
            Some(&reservation),
            CapabilityInvocation { input: json!({}) },
            Arc::new(read_write_fs),
        )
        .unwrap();

    assert_eq!(result.output, json!({"ok": true}));
    assert_eq!(
        std::fs::read_to_string(storage.path().join("project/generated.json")).unwrap(),
        r#"{"created":true}"#
    );

    let (read_only_fs, storage) = scoped_filesystem(MountPermissions::read_only(), |_| {});
    let result = runtime
        .invoke_json_with_filesystem(
            &module,
            &descriptor,
            Some(&reservation),
            CapabilityInvocation { input: json!({}) },
            Arc::new(read_only_fs),
        )
        .unwrap();

    assert_eq!(result.output, json!({"ok": false}));
    assert!(!storage.path().join("project/generated.json").exists());
}

#[test]
fn fs_list_and_stat_imports_use_scoped_filesystem() {
    let (wasm_fs, _storage) = scoped_filesystem(MountPermissions::read_only(), |project| {
        std::fs::write(project.join("a.json"), b"{}").unwrap();
        std::fs::write(project.join("b.json"), b"{}").unwrap();
    });
    let runtime = WasmRuntime::for_testing().unwrap();
    let module = runtime.prepare(fs_list_stat_spec()).unwrap();
    let descriptor = make_descriptor("fs-demo", "fs-demo.list_stat", RuntimeKind::Wasm);
    let reservation = sample_reservation();

    let result = runtime
        .invoke_json_with_filesystem(
            &module,
            &descriptor,
            Some(&reservation),
            CapabilityInvocation { input: json!({}) },
            Arc::new(wasm_fs),
        )
        .unwrap();

    assert_eq!(result.output, json!(["a.json", "b.json"]));
}

#[test]
fn fs_import_rejects_oversized_path_before_filesystem_call() {
    let recording_fs = RecordingFilesystem::default();
    let runtime = WasmRuntime::for_testing().unwrap();
    let oversized_path = format!("/workspace/{}", "a".repeat(5 * 1024));
    let module = runtime.prepare(fs_read_path_spec(&oversized_path)).unwrap();
    let descriptor = make_descriptor("fs-demo", "fs-demo.read", RuntimeKind::Wasm);
    let reservation = sample_reservation();

    let result = runtime
        .invoke_json_with_filesystem(
            &module,
            &descriptor,
            Some(&reservation),
            CapabilityInvocation { input: json!({}) },
            Arc::new(recording_fs.clone()),
        )
        .unwrap();

    assert_eq!(result.output, json!({"ok": false}));
    assert!(recording_fs.read_paths.lock().unwrap().is_empty());
}

#[test]
fn scoped_filesystem_bridge_polls_root_filesystem_off_calling_thread() {
    let caller_thread = std::thread::current().id();
    let root = Arc::new(ThreadRecordingRootFilesystem::default());
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/workspace").unwrap(),
        VirtualPath::new("/projects/project").unwrap(),
        MountPermissions::read_only(),
    )])
    .unwrap();
    let wasm_fs = WasmScopedFilesystem::new(root.clone(), mounts);

    let contents = wasm_fs.read_utf8("/workspace/input.json").unwrap();

    assert_eq!(contents, r#"{"ok":true}"#);
    let observed_thread = root.observed_read_thread.lock().unwrap().unwrap();
    assert_ne!(observed_thread, caller_thread);
}

fn scoped_filesystem(
    permissions: MountPermissions,
    populate: impl FnOnce(&std::path::Path),
) -> (WasmScopedFilesystem<LocalFilesystem>, tempfile::TempDir) {
    let storage = tempdir().unwrap();
    let project = storage.path().join("project");
    std::fs::create_dir_all(&project).unwrap();
    populate(&project);

    let mut root = LocalFilesystem::new();
    root.mount_local(
        VirtualPath::new("/projects").unwrap(),
        HostPath::from_path_buf(storage.path().to_path_buf()),
    )
    .unwrap();
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/workspace").unwrap(),
        VirtualPath::new("/projects/project").unwrap(),
        permissions,
    )])
    .unwrap();

    (WasmScopedFilesystem::new(Arc::new(root), mounts), storage)
}

#[derive(Clone, Default)]
struct RecordingFilesystem {
    read_paths: Arc<Mutex<Vec<String>>>,
}

impl WasmHostFilesystem for RecordingFilesystem {
    fn read_utf8(&self, path: &str) -> Result<String, String> {
        self.read_paths.lock().unwrap().push(path.to_string());
        Ok(r#"{"ok":true}"#.to_string())
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

#[derive(Debug, Default)]
struct ThreadRecordingRootFilesystem {
    observed_read_thread: Mutex<Option<ThreadId>>,
}

#[async_trait]
impl RootFilesystem for ThreadRecordingRootFilesystem {
    async fn read_file(&self, _path: &VirtualPath) -> Result<Vec<u8>, FilesystemError> {
        *self.observed_read_thread.lock().unwrap() = Some(std::thread::current().id());
        Ok(br#"{"ok":true}"#.to_vec())
    }

    async fn write_file(&self, path: &VirtualPath, _bytes: &[u8]) -> Result<(), FilesystemError> {
        Err(backend_error(path, FilesystemOperation::WriteFile))
    }

    async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
        Err(backend_error(path, FilesystemOperation::ListDir))
    }

    async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
        Err(backend_error(path, FilesystemOperation::Stat))
    }
}

fn backend_error(path: &VirtualPath, operation: FilesystemOperation) -> FilesystemError {
    FilesystemError::Backend {
        path: path.clone(),
        operation,
        reason: "not implemented by test filesystem".to_string(),
    }
}

fn fs_read_path_spec(path: &str) -> WasmModuleSpec {
    let path_len = path.len();
    WasmModuleSpec {
        provider: ExtensionId::new("fs-demo").unwrap(),
        capability: CapabilityId::new("fs-demo.read").unwrap(),
        export: "read".to_string(),
        bytes: wat::parse_str(format!(
            r#"(module
                (import "host" "fs_read_utf8" (func $read (param i32 i32 i32 i32) (result i32)))
                (memory (export "memory") 1)
                (data (i32.const 64) "{path}")
                (data (i32.const 20000) "{{\"ok\":false}}")
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
                (func (export "read") (param i32 i32) (result i32)
                  (local $n i32)
                  i32.const 64
                  i32.const {path_len}
                  i32.const 32768
                  i32.const 512
                  call $read
                  local.set $n
                  local.get $n
                  i32.const 0
                  i32.ge_s
                  if
                    i32.const 32768
                    global.set $out_ptr
                    local.get $n
                    global.set $out_len
                  else
                    i32.const 20000
                    global.set $out_ptr
                    i32.const 12
                    global.set $out_len
                  end
                  i32.const 0)
                (func (export "output_ptr") (result i32) global.get $out_ptr)
                (func (export "output_len") (result i32) global.get $out_len))"#,
        ))
        .unwrap(),
    }
}

fn fs_read_spec() -> WasmModuleSpec {
    WasmModuleSpec {
        provider: ExtensionId::new("fs-demo").unwrap(),
        capability: CapabilityId::new("fs-demo.read").unwrap(),
        export: "read".to_string(),
        bytes: wat::parse_str(
            r#"(module
                (import "host" "fs_read_utf8" (func $read (param i32 i32 i32 i32) (result i32)))
                (memory (export "memory") 1)
                (data (i32.const 64) "/workspace/input.json")
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
                (func (export "read") (param i32 i32) (result i32)
                  (local $n i32)
                  i32.const 64
                  i32.const 21
                  i32.const 4096
                  i32.const 512
                  call $read
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
                (func (export "output_len") (result i32) global.get $out_len))"#,
        )
        .unwrap(),
    }
}

fn fs_write_spec() -> WasmModuleSpec {
    WasmModuleSpec {
        provider: ExtensionId::new("fs-demo").unwrap(),
        capability: CapabilityId::new("fs-demo.write").unwrap(),
        export: "write".to_string(),
        bytes: wat::parse_str(
            r#"(module
                (import "host" "fs_write_utf8" (func $write (param i32 i32 i32 i32) (result i32)))
                (memory (export "memory") 1)
                (data (i32.const 64) "/workspace/generated.json")
                (data (i32.const 128) "{\"created\":true}")
                (data (i32.const 256) "{\"ok\":true}")
                (data (i32.const 288) "{\"ok\":false}")
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
                (func (export "write") (param i32 i32) (result i32)
                  (local $status i32)
                  i32.const 64
                  i32.const 25
                  i32.const 128
                  i32.const 16
                  call $write
                  local.set $status
                  local.get $status
                  i32.eqz
                  if
                    i32.const 256
                    global.set $out_ptr
                    i32.const 11
                    global.set $out_len
                  else
                    i32.const 288
                    global.set $out_ptr
                    i32.const 12
                    global.set $out_len
                  end
                  i32.const 0)
                (func (export "output_ptr") (result i32) global.get $out_ptr)
                (func (export "output_len") (result i32) global.get $out_len))"#,
        )
        .unwrap(),
    }
}

fn fs_list_stat_spec() -> WasmModuleSpec {
    WasmModuleSpec {
        provider: ExtensionId::new("fs-demo").unwrap(),
        capability: CapabilityId::new("fs-demo.list_stat").unwrap(),
        export: "list_stat".to_string(),
        bytes: wat::parse_str(
            r#"(module
                (import "host" "fs_list_utf8" (func $list (param i32 i32 i32 i32) (result i32)))
                (import "host" "fs_stat_len" (func $stat (param i32 i32) (result i64)))
                (memory (export "memory") 1)
                (data (i32.const 64) "/workspace")
                (data (i32.const 96) "/workspace/a.json")
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
                (func (export "list_stat") (param i32 i32) (result i32)
                  (local $n i32)
                  i32.const 96
                  i32.const 17
                  call $stat
                  i64.const 0
                  i64.ge_s
                  if
                    i32.const 64
                    i32.const 10
                    i32.const 4096
                    i32.const 512
                    call $list
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
                  else
                    i32.const 256
                    global.set $out_ptr
                    i32.const 12
                    global.set $out_len
                  end
                  i32.const 0)
                (func (export "output_ptr") (result i32) global.get $out_ptr)
                (func (export "output_len") (result i32) global.get $out_len))"#,
        )
        .unwrap(),
    }
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
