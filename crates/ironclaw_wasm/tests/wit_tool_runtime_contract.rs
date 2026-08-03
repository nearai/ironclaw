use std::sync::Arc;

use ironclaw_wasm::{
    DenyWasmHostHttp, RecordingWasmHostHttp, WASM_DIAGNOSTIC_MAX_BYTES,
    WASM_DIAGNOSTIC_MAX_ENTRIES_PER_EXECUTION, WASM_DIAGNOSTIC_REDACTION_MARKER, WasmError,
    WasmHostHttp, WasmHttpRequest, WasmHttpResponse, WitToolHost, WitToolRequest, WitToolRuntime,
    WitToolRuntimeConfig,
};
use serde_json::json;
use wit_component::{ComponentEncoder, StringEncoding, embed_component_metadata};
use wit_parser::Resolve;

const COUNTER_TOOL_WAT: &str = r#"
(module
  (type (;0;) (func (param i32 i32 i32)))
  (type (;1;) (func (result i64)))
  (type (;2;) (func (param i32 i32 i32 i32 i32 i32 i32 i32 i32 i32 i32 i32)))
  (type (;3;) (func (param i32 i32 i32 i32 i32)))
  (type (;4;) (func (param i32 i32) (result i32)))
  (import "near:agent/host@0.3.0" "log" (func $log (type 0)))
  (import "near:agent/host@0.3.0" "now-millis" (func $now (type 1)))
  (import "near:agent/host@0.3.0" "workspace-read" (func $workspace_read (type 0)))
  (import "near:agent/host@0.3.0" "http-request" (func $http_request (type 2)))
  (import "near:agent/host@0.3.0" "tool-invoke" (func $tool_invoke (type 3)))
  (import "near:agent/host@0.3.0" "secret-exists" (func $secret_exists (type 4)))
  (memory (export "memory") 1)
  (global $heap (mut i32) (i32.const 4096))
  (global $count (mut i32) (i32.const 0))
  (data (i32.const 1024) "{\22type\22:\22object\22}")
  (data (i32.const 2048) "fixture description")
  (data (i32.const 3072) "1")
  (data (i32.const 3073) "2")
  (func $schema (result i32)
    i32.const 16
    i32.const 1024
    i32.store
    i32.const 20
    i32.const 17
    i32.store
    i32.const 16)
  (func $description (result i32)
    i32.const 32
    i32.const 2048
    i32.store
    i32.const 36
    i32.const 19
    i32.store
    i32.const 32)
  (func $execute (param i32 i32 i32 i32 i32) (result i32)
    global.get $count
    i32.const 1
    i32.add
    global.set $count

    i32.const 48
    i32.const 1
    i32.store
    i32.const 52
    global.get $count
    i32.const 1
    i32.eq
    if (result i32)
      i32.const 3072
    else
      i32.const 3073
    end
    i32.store
    i32.const 56
    i32.const 1
    i32.store
    i32.const 60
    i32.const 0
    i32.store
    i32.const 48)
  (func $post (param i32))
  (func $realloc (param $old i32) (param $old_align i32) (param $new_size i32) (param $new_align i32) (result i32)
    (local $ret i32)
    global.get $heap
    local.set $ret
    global.get $heap
    local.get $new_size
    i32.add
    global.set $heap
    local.get $ret)
  (func $_initialize)
  (export "near:agent/tool@0.3.0#execute" (func $execute))
  (export "cabi_post_near:agent/tool@0.3.0#execute" (func $post))
  (export "near:agent/tool@0.3.0#schema" (func $schema))
  (export "cabi_post_near:agent/tool@0.3.0#schema" (func $post))
  (export "near:agent/tool@0.3.0#description" (func $description))
  (export "cabi_post_near:agent/tool@0.3.0#description" (func $post))
  (export "cabi_realloc" (func $realloc))
  (export "_initialize" (func $_initialize))
)
"#;

const HTTP_TOOL_WAT: &str = r#"
(module
  (type (;0;) (func (param i32 i32 i32)))
  (type (;1;) (func (result i64)))
  (type (;2;) (func (param i32 i32 i32 i32 i32 i32 i32 i32 i32 i32 i32 i32)))
  (type (;3;) (func (param i32 i32 i32 i32 i32)))
  (type (;4;) (func (param i32 i32) (result i32)))
  (import "near:agent/host@0.3.0" "log" (func $log (type 0)))
  (import "near:agent/host@0.3.0" "now-millis" (func $now (type 1)))
  (import "near:agent/host@0.3.0" "workspace-read" (func $workspace_read (type 0)))
  (import "near:agent/host@0.3.0" "http-request" (func $http_request (type 2)))
  (import "near:agent/host@0.3.0" "tool-invoke" (func $tool_invoke (type 3)))
  (import "near:agent/host@0.3.0" "secret-exists" (func $secret_exists (type 4)))
  (memory (export "memory") 1)
  (global $heap (mut i32) (i32.const 4096))
  (data (i32.const 128) "POST")
  (data (i32.const 160) "https://example.test/api")
  (data (i32.const 224) "{}")
  (data (i32.const 256) "hello")
  (data (i32.const 1024) "{\22type\22:\22object\22}")
  (data (i32.const 2048) "fixture description")
  (data (i32.const 3072) "1")
  (func $schema (result i32)
    i32.const 16
    i32.const 1024
    i32.store
    i32.const 20
    i32.const 17
    i32.store
    i32.const 16)
  (func $description (result i32)
    i32.const 32
    i32.const 2048
    i32.store
    i32.const 36
    i32.const 19
    i32.store
    i32.const 32)
  (func $execute (param i32 i32 i32 i32 i32) (result i32)
    i32.const 128
    i32.const 4
    i32.const 160
    i32.const 24
    i32.const 224
    i32.const 2
    i32.const 1
    i32.const 256
    i32.const 5
    i32.const 0
    i32.const 0
    i32.const 512
    call $http_request

    i32.const 48
    i32.const 1
    i32.store
    i32.const 52
    i32.const 3072
    i32.store
    i32.const 56
    i32.const 1
    i32.store
    i32.const 60
    i32.const 0
    i32.store
    i32.const 48)
  (func $post (param i32))
  (func $realloc (param $old i32) (param $old_align i32) (param $new_size i32) (param $new_align i32) (result i32)
    (local $ret i32)
    global.get $heap
    local.set $ret
    global.get $heap
    local.get $new_size
    i32.add
    global.set $heap
    local.get $ret)
  (func $_initialize)
  (export "near:agent/tool@0.3.0#execute" (func $execute))
  (export "cabi_post_near:agent/tool@0.3.0#execute" (func $post))
  (export "near:agent/tool@0.3.0#schema" (func $schema))
  (export "cabi_post_near:agent/tool@0.3.0#schema" (func $post))
  (export "near:agent/tool@0.3.0#description" (func $description))
  (export "cabi_post_near:agent/tool@0.3.0#description" (func $post))
  (export "cabi_realloc" (func $realloc))
  (export "_initialize" (func $_initialize))
)
"#;

fn tool_component(wat_src: &str) -> Vec<u8> {
    let mut module = wat::parse_str(wat_src).expect("fixture WAT must parse");
    let mut resolve = Resolve::default();
    let package = resolve
        .push_str("tool.wit", ironclaw_wasm::TOOL_WIT)
        .expect("tool WIT must parse");
    let world = resolve
        .select_world(&[package], Some("sandboxed-tool"))
        .expect("sandboxed-tool world must exist");

    embed_component_metadata(&mut module, &resolve, world, StringEncoding::UTF8)
        .expect("component metadata must embed");

    let mut encoder = ComponentEncoder::default()
        .module(&module)
        .expect("fixture module must decode")
        .validate(true);
    encoder.encode().expect("component must encode")
}

#[test]
fn prepares_metadata_from_wit_tool_component() {
    let runtime = WitToolRuntime::new(WitToolRuntimeConfig::for_testing()).unwrap();
    let prepared = runtime
        .prepare("counter", &tool_component(COUNTER_TOOL_WAT))
        .unwrap();

    assert_eq!(prepared.name(), "counter");
    assert_eq!(prepared.description(), "fixture description");
    assert_eq!(prepared.schema(), &json!({ "type": "object" }));
}

#[test]
fn malformed_component_bytes_are_rejected_as_compilation_failure() {
    let runtime = WitToolRuntime::new(WitToolRuntimeConfig::for_testing()).unwrap();

    let error = runtime
        .prepare("malformed", b"not a wasm component")
        .unwrap_err();

    assert!(
        matches!(error, WasmError::CompilationFailed(_)),
        "unexpected error: {error:?}"
    );
}

#[test]
fn core_wasm_module_bytes_are_rejected_as_compilation_failure() {
    let runtime = WitToolRuntime::new(WitToolRuntimeConfig::for_testing()).unwrap();
    let core_module = wat::parse_str("(module)").unwrap();

    let error = runtime.prepare("core-module", &core_module).unwrap_err();

    assert!(
        matches!(error, WasmError::CompilationFailed(_)),
        "unexpected error: {error:?}"
    );
}

#[test]
fn unsupported_component_without_tool_exports_is_rejected_at_instantiation() {
    let runtime = WitToolRuntime::new(WitToolRuntimeConfig::for_testing()).unwrap();
    let component_without_tool_exports = wat::parse_str("(component)").unwrap();

    let error = runtime
        .prepare("unsupported", &component_without_tool_exports)
        .unwrap_err();

    assert!(
        matches!(error, WasmError::InstantiationFailed(_)),
        "unexpected error: {error:?}"
    );
}

#[test]
fn schema_export_must_return_json_object() {
    let runtime = WitToolRuntime::new(WitToolRuntimeConfig::for_testing()).unwrap();
    let invalid_schema_wat = COUNTER_TOOL_WAT
        .replace(
            r#"(data (i32.const 1024) "{\22type\22:\22object\22}")"#,
            r#"(data (i32.const 1024) "[1]")"#,
        )
        .replace(
            "i32.const 17\n    i32.store\n    i32.const 16)\n  (func $description",
            "i32.const 3\n    i32.store\n    i32.const 16)\n  (func $description",
        );
    assert_ne!(
        invalid_schema_wat, COUNTER_TOOL_WAT,
        "invalid schema WAT mutation should match the fixture"
    );

    let error = runtime
        .prepare("invalid-schema", &tool_component(&invalid_schema_wat))
        .unwrap_err();

    assert!(
        matches!(error, WasmError::InvalidSchema(_)),
        "unexpected error: {error:?}"
    );
}

#[test]
fn executes_wit_tool_with_fresh_component_instance_per_call() {
    let runtime = WitToolRuntime::new(WitToolRuntimeConfig::for_testing()).unwrap();
    let prepared = runtime
        .prepare("counter", &tool_component(COUNTER_TOOL_WAT))
        .unwrap();
    let host = WitToolHost::deny_all();

    let first = runtime
        .execute(&prepared, host.clone(), WitToolRequest::new(r#"{"q":1}"#))
        .unwrap();
    let second = runtime
        .execute(&prepared, host, WitToolRequest::new(r#"{"q":2}"#))
        .unwrap();

    assert_eq!(first.output_json.as_deref(), Some("1"));
    assert_eq!(second.output_json.as_deref(), Some("1"));
    assert!(first.error.is_none());
    assert!(second.error.is_none());
}

// Regression: the host runtime offloads `WitToolRuntime::execute` to the
// blocking thread pool via `tokio::task::spawn_blocking` so a synchronous
// wasmtime guest call never parks an async worker (the runtime wedge). That
// requires a clone of the runtime to be `Send + 'static` and to execute
// correctly off-thread. This test exercises exactly that contract: clone the
// runtime, move the clone + prepared component into `spawn_blocking`, and run
// several concurrently.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cloned_runtime_executes_inside_spawn_blocking_concurrently() {
    let runtime = Arc::new(WitToolRuntime::new(WitToolRuntimeConfig::for_testing()).unwrap());
    let prepared = Arc::new(
        runtime
            .prepare("counter", &tool_component(COUNTER_TOOL_WAT))
            .unwrap(),
    );

    let mut handles = Vec::new();
    for _ in 0..8 {
        let runtime = Arc::clone(&runtime);
        let prepared = Arc::clone(&prepared);
        handles.push(tokio::task::spawn_blocking(move || {
            let host = WitToolHost::deny_all();
            runtime.execute(&prepared, host, WitToolRequest::new(r#"{"q":1}"#))
        }));
    }

    for handle in handles {
        let execution = handle.await.expect("blocking task must not panic").unwrap();
        assert_eq!(execution.output_json.as_deref(), Some("1"));
        assert!(execution.error.is_none());
    }
}

#[test]
fn http_import_delegates_to_recording_host_and_counts_request_body_only() {
    let runtime = WitToolRuntime::new(WitToolRuntimeConfig::for_testing()).unwrap();
    let prepared = runtime
        .prepare("http", &tool_component(HTTP_TOOL_WAT))
        .unwrap();
    let http = Arc::new(RecordingWasmHostHttp::ok(WasmHttpResponse {
        status: 201,
        headers_json: r#"{"content-type":"text/plain"}"#.to_string(),
        body: b"response body is not egress".to_vec(),
    }));
    let host = WitToolHost::deny_all().with_http(http.clone());

    let executed = runtime
        .execute(&prepared, host, WitToolRequest::new("{}"))
        .unwrap();

    let requests = http.requests().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, "POST");
    assert_eq!(requests[0].url, "https://example.test/api");
    assert_eq!(requests[0].headers_json, "{}");
    assert_eq!(requests[0].body.as_deref(), Some(&b"hello"[..]));
    assert_eq!(executed.usage.network_egress_bytes, 5);
}

#[test]
fn http_import_counts_request_body_when_host_reports_failure_after_send() {
    let runtime = WitToolRuntime::new(WitToolRuntimeConfig::for_testing()).unwrap();
    let prepared = runtime
        .prepare("http", &tool_component(HTTP_TOOL_WAT))
        .unwrap();
    let http = Arc::new(RecordingWasmHostHttp::err(
        ironclaw_wasm::WasmHostError::FailedAfterRequestSent(
            "response body limit exceeded".to_string(),
        ),
    ));
    let host = WitToolHost::deny_all().with_http(http.clone());

    let executed = runtime
        .execute(&prepared, host, WitToolRequest::new("{}"))
        .unwrap();

    let requests = http.requests().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].body.as_deref(), Some(&b"hello"[..]));
    assert_eq!(executed.usage.network_egress_bytes, 5);
}

#[test]
fn default_http_host_fails_closed_without_recording_egress() {
    let denied = DenyWasmHostHttp
        .request(WasmHttpRequest {
            method: "GET".to_string(),
            url: "https://example.test/".to_string(),
            headers_json: "{}".to_string(),
            body: Some(b"should-not-send".to_vec()),
            timeout_ms: None,
        })
        .unwrap_err();
    assert!(denied.to_string().contains("not configured"));

    let runtime = WitToolRuntime::new(WitToolRuntimeConfig::for_testing()).unwrap();
    let prepared = runtime
        .prepare("http", &tool_component(HTTP_TOOL_WAT))
        .unwrap();
    let executed = runtime
        .execute(
            &prepared,
            WitToolHost::deny_all(),
            WitToolRequest::new("{}"),
        )
        .unwrap();

    assert_eq!(executed.usage.network_egress_bytes, 0);
}

#[test]
fn execution_error_preserves_usage_when_guest_traps_after_host_egress() {
    let runtime = WitToolRuntime::new(WitToolRuntimeConfig::for_testing()).unwrap();
    let prepared = runtime
        .prepare("http", &tool_component(&trap_after_http_wat()))
        .unwrap();
    let http = Arc::new(RecordingWasmHostHttp::ok(WasmHttpResponse {
        status: 201,
        headers_json: "{}".to_string(),
        body: Vec::new(),
    }));
    let host = WitToolHost::deny_all().with_http(http.clone());

    let error = runtime
        .execute(&prepared, host, WitToolRequest::new("{}"))
        .unwrap_err();

    assert_eq!(http.requests().unwrap().len(), 1);
    match error {
        ironclaw_wasm::WasmError::ExecutionFailed { usage, .. } => {
            assert_eq!(usage.network_egress_bytes, 5);
        }
        other => panic!("expected execution failure with usage, got {other:?}"),
    }
}

#[test]
fn allows_multiple_linear_memories_within_aggregate_memory_budget() {
    let runtime = WitToolRuntime::new(WitToolRuntimeConfig {
        default_limits: ironclaw_wasm::wasm_sandbox_core::SandboxLimits::default()
            .with_memory_bytes(128 * 1024)
            .with_fuel(100_000)
            .with_timeout(std::time::Duration::from_secs(5)),
    })
    .unwrap();
    let multi_memory = COUNTER_TOOL_WAT.replace(
        "(memory (export \"memory\") 1)",
        "(memory (export \"memory\") 1)\n  (memory 1)",
    );

    let prepared = runtime
        .prepare("counter", &tool_component(&multi_memory))
        .unwrap();

    assert_eq!(prepared.name(), "counter");
}

#[test]
fn rejects_multiple_linear_memories_that_exceed_aggregate_memory_budget() {
    let runtime = WitToolRuntime::new(WitToolRuntimeConfig {
        default_limits: ironclaw_wasm::wasm_sandbox_core::SandboxLimits::default()
            .with_memory_bytes(64 * 1024)
            .with_fuel(100_000)
            .with_timeout(std::time::Duration::from_secs(5)),
    })
    .unwrap();
    let multi_memory = COUNTER_TOOL_WAT.replace(
        "(memory (export \"memory\") 1)",
        "(memory (export \"memory\") 1)\n  (memory 1)",
    );

    let result = runtime.prepare("counter", &tool_component(&multi_memory));

    assert!(
        result.is_err(),
        "memory_bytes should be enforced across all component memories"
    );
}

#[test]
fn http_import_caps_guest_timeout_to_remaining_execution_deadline() {
    use std::sync::Mutex;

    #[derive(Debug, Default)]
    struct CapturingHttp {
        timeout_ms: Mutex<Option<u32>>,
    }

    impl WasmHostHttp for CapturingHttp {
        fn request(
            &self,
            request: WasmHttpRequest,
        ) -> Result<WasmHttpResponse, ironclaw_wasm::WasmHostError> {
            *self.timeout_ms.lock().unwrap() = request.timeout_ms;
            Ok(WasmHttpResponse {
                status: 200,
                headers_json: "{}".to_string(),
                body: Vec::new(),
            })
        }
    }

    let runtime = WitToolRuntime::new(WitToolRuntimeConfig::for_testing()).unwrap();
    let prepared = runtime
        .prepare("http", &tool_component(HTTP_TOOL_WAT))
        .unwrap();
    let http = Arc::new(CapturingHttp::default());
    let host = WitToolHost::deny_all().with_http(http.clone());

    runtime
        .execute(&prepared, host, WitToolRequest::new("{}"))
        .unwrap();

    let timeout_ms = http.timeout_ms.lock().unwrap().expect("timeout is capped");
    assert!(
        timeout_ms <= 5_000,
        "host timeout should be capped to the execution deadline, got {timeout_ms}ms"
    );
}

#[test]
fn guest_trap_after_overdue_host_import_reports_deadline_and_preserves_usage() {
    use std::time::Duration;

    struct SlowHttp;

    impl WasmHostHttp for SlowHttp {
        fn request(
            &self,
            _request: WasmHttpRequest,
        ) -> Result<WasmHttpResponse, ironclaw_wasm::WasmHostError> {
            std::thread::sleep(Duration::from_millis(50));
            Ok(WasmHttpResponse {
                status: 200,
                headers_json: "{}".to_string(),
                body: Vec::new(),
            })
        }
    }

    let runtime = WitToolRuntime::new(WitToolRuntimeConfig {
        default_limits: ironclaw_wasm::wasm_sandbox_core::SandboxLimits::default()
            .with_memory_bytes(1024 * 1024)
            .with_fuel(100_000)
            .with_timeout(Duration::from_millis(20)),
    })
    .unwrap();
    let prepared = runtime
        .prepare("http", &tool_component(&trap_after_http_wat()))
        .unwrap();
    let host = WitToolHost::deny_all().with_http(Arc::new(SlowHttp));

    let error = runtime
        .execute(&prepared, host, WitToolRequest::new("{}"))
        .unwrap_err();

    assert!(
        error.to_string().contains("deadline"),
        "unexpected error: {error}"
    );
    match error {
        ironclaw_wasm::WasmError::ExecutionFailed { usage, .. } => {
            assert_eq!(usage.network_egress_bytes, 5);
        }
        other => panic!("expected execution failure with usage, got {other:?}"),
    }
}

#[test]
fn http_import_uses_wit_default_when_guest_omits_timeout_below_execution_deadline() {
    use std::sync::Mutex;

    #[derive(Debug, Default)]
    struct CapturingHttp {
        timeout_ms: Mutex<Option<u32>>,
    }

    impl WasmHostHttp for CapturingHttp {
        fn request(
            &self,
            request: WasmHttpRequest,
        ) -> Result<WasmHttpResponse, ironclaw_wasm::WasmHostError> {
            *self.timeout_ms.lock().unwrap() = request.timeout_ms;
            Ok(WasmHttpResponse {
                status: 200,
                headers_json: "{}".to_string(),
                body: Vec::new(),
            })
        }
    }

    let runtime = WitToolRuntime::new(WitToolRuntimeConfig::default()).unwrap();
    let prepared = runtime
        .prepare("http", &tool_component(HTTP_TOOL_WAT))
        .unwrap();
    let http = Arc::new(CapturingHttp::default());
    let host = WitToolHost::deny_all().with_http(http.clone());

    runtime
        .execute(&prepared, host, WitToolRequest::new("{}"))
        .unwrap();

    assert_eq!(*http.timeout_ms.lock().unwrap(), Some(30_000));
}

#[test]
fn execution_fails_when_host_import_returns_after_deadline() {
    use std::time::Duration;

    struct SlowHttp;

    impl WasmHostHttp for SlowHttp {
        fn request(
            &self,
            _request: WasmHttpRequest,
        ) -> Result<WasmHttpResponse, ironclaw_wasm::WasmHostError> {
            std::thread::sleep(Duration::from_millis(50));
            Ok(WasmHttpResponse {
                status: 200,
                headers_json: "{}".to_string(),
                body: Vec::new(),
            })
        }
    }

    let runtime = WitToolRuntime::new(WitToolRuntimeConfig {
        default_limits: ironclaw_wasm::wasm_sandbox_core::SandboxLimits::default()
            .with_memory_bytes(1024 * 1024)
            .with_fuel(100_000)
            .with_timeout(Duration::from_millis(20)),
    })
    .unwrap();
    let prepared = runtime
        .prepare("http", &tool_component(HTTP_TOOL_WAT))
        .unwrap();
    let host = WitToolHost::deny_all().with_http(Arc::new(SlowHttp));

    let error = runtime
        .execute(&prepared, host, WitToolRequest::new("{}"))
        .unwrap_err();

    assert!(
        error.to_string().contains("deadline"),
        "unexpected error: {error}"
    );
}

fn trap_after_http_wat() -> String {
    HTTP_TOOL_WAT.replace(
        "i32.const 48\n    i32.const 1\n    i32.store",
        "unreachable\n\n    i32.const 48\n    i32.const 1\n    i32.store",
    )
}

fn wat_bytes(value: &str) -> String {
    value
        .as_bytes()
        .iter()
        .map(|byte| format!("\\{byte:02x}"))
        .collect()
}

fn diagnostic_tool_wat(logs: &[(u32, String)], response_error: Option<&str>, trap: bool) -> String {
    let mut wat = COUNTER_TOOL_WAT.to_string();
    let mut data = String::new();
    let mut calls = String::new();
    let mut next_offset = 8_192_u32;
    let mut message_offsets = std::collections::BTreeMap::new();

    for (level, message) in logs {
        let message_offset = if let Some(offset) = message_offsets.get(message) {
            *offset
        } else {
            let offset = next_offset;
            data.push_str(&format!(
                "  (data (i32.const {offset}) \"{}\")\n",
                wat_bytes(message)
            ));
            next_offset = next_offset
                .checked_add(
                    u32::try_from(message.len()).expect("fixture message length must fit u32"),
                )
                .and_then(|next| next.checked_add(16))
                .expect("fixture data offsets must not overflow");
            message_offsets.insert(message.clone(), offset);
            offset
        };
        calls.push_str(&format!(
            "    i32.const {level}\n    i32.const {message_offset}\n    i32.const {}\n    call $log\n",
            message.len()
        ));
    }

    if let Some(error) = response_error {
        data.push_str(&format!(
            "  (data (i32.const {next_offset}) \"{}\")\n",
            wat_bytes(error)
        ));
    }

    wat = wat.replacen("  (func $schema", &format!("{data}  (func $schema"), 1);
    wat = wat.replacen(
        "  (func $execute (param i32 i32 i32 i32 i32) (result i32)\n",
        &format!(
            "  (func $execute (param i32 i32 i32 i32 i32) (result i32)\n{calls}{}",
            if trap { "    unreachable\n" } else { "" }
        ),
        1,
    );

    if let Some(error) = response_error {
        let success_response = r#"    i32.const 48
    i32.const 1
    i32.store
    i32.const 52
    global.get $count
    i32.const 1
    i32.eq
    if (result i32)
      i32.const 3072
    else
      i32.const 3073
    end
    i32.store
    i32.const 56
    i32.const 1
    i32.store
    i32.const 60
    i32.const 0
    i32.store
    i32.const 48)"#;
        let error_response = format!(
            r#"    i32.const 48
    i32.const 0
    i32.store
    i32.const 60
    i32.const 1
    i32.store
    i32.const 64
    i32.const {next_offset}
    i32.store
    i32.const 68
    i32.const {}
    i32.store
    i32.const 48)"#,
            error.len()
        );
        wat = wat.replacen(success_response, &error_response, 1);
    }

    wat
}

fn execute_diagnostic_tool(
    logs: &[(u32, String)],
    response_error: Option<&str>,
    trap: bool,
) -> Result<ironclaw_wasm::WitToolExecution, WasmError> {
    let runtime = WitToolRuntime::new(WitToolRuntimeConfig::for_testing()).unwrap();
    let prepared = runtime
        .prepare(
            "diagnostic-boundary",
            &tool_component(&diagnostic_tool_wat(logs, response_error, trap)),
        )
        .unwrap();
    runtime.execute(
        &prepared,
        WitToolHost::deny_all(),
        WitToolRequest::new("{}"),
    )
}

fn secret_patterns() -> [(String, &'static str); 3] {
    [
        (
            format!("sk-{}", "B".repeat(24)),
            "block-action API-key shape",
        ),
        (
            format!("Bearer {}", "r".repeat(24)),
            "redact-action bearer shape",
        ),
        (
            "0123456789abcdef".repeat(4),
            "warn-action high-entropy-hex shape",
        ),
    ]
}

#[test]
fn guest_logs_sanitize_every_leak_action_before_public_success_result() {
    let mut logs = vec![(2, "benign diagnostic".to_string())];
    for (secret, label) in secret_patterns() {
        logs.push((3, format!("{label}: {secret}; retained cause")));
    }

    let executed = execute_diagnostic_tool(&logs, None, false).unwrap();

    assert_eq!(executed.logs[0].message, "benign diagnostic");
    for (record, (secret, label)) in executed.logs[1..].iter().zip(secret_patterns()) {
        assert!(!record.message.contains(&secret), "{label} leaked");
        assert!(
            record.message.contains("retained cause"),
            "sanitization should retain non-secret diagnostic context"
        );
        assert!(record.message.contains(WASM_DIAGNOSTIC_REDACTION_MARKER));
    }
}

#[test]
fn guest_response_error_is_sanitized_independently_from_captured_logs() {
    let log_secret = format!("sk-{}", "L".repeat(24));
    let response_secret = format!("Bearer {}", "e".repeat(24));
    let response_error = format!("request failed for {response_secret}; status=503");

    let executed = execute_diagnostic_tool(
        &[(4, format!("log contained {log_secret}; log cause"))],
        Some(&response_error),
        false,
    )
    .unwrap();

    let error = executed
        .error
        .expect("guest response.error must be retained");
    assert!(!error.contains(&response_secret));
    assert!(error.contains("status=503"));
    assert!(error.contains(WASM_DIAGNOSTIC_REDACTION_MARKER));
    assert!(!executed.logs[0].message.contains(&log_secret));
    assert!(executed.logs[0].message.contains("log cause"));
}

#[test]
fn guest_response_error_size_boundary_is_fail_closed() {
    let retained_cause = "guest returned status=503; ";
    let exactly_at_limit = format!(
        "{retained_cause}{}",
        "r".repeat(WASM_DIAGNOSTIC_MAX_BYTES - retained_cause.len())
    );
    let over_limit = "e".repeat(WASM_DIAGNOSTIC_MAX_BYTES + 1);

    let retained = execute_diagnostic_tool(&[], Some(&exactly_at_limit), false).unwrap();
    assert_eq!(retained.error.as_deref(), Some(exactly_at_limit.as_str()));

    let redacted = execute_diagnostic_tool(&[], Some(&over_limit), false).unwrap();
    assert_eq!(
        redacted.error.as_deref(),
        Some(WASM_DIAGNOSTIC_REDACTION_MARKER)
    );
}

#[test]
fn guest_trap_preserves_sanitized_log_snapshot_and_safe_trap_cause() {
    let secret = format!("sk-{}", "T".repeat(24));
    let guest_function_name = "guest-owned-private-execute-marker";
    let component = diagnostic_tool_wat(
        &[(4, format!("before trap {secret}; operation=write"))],
        None,
        true,
    )
    .replace("$execute", &format!("${guest_function_name}"));
    let runtime = WitToolRuntime::new(WitToolRuntimeConfig::for_testing()).unwrap();
    let prepared = runtime
        .prepare("diagnostic-boundary", &tool_component(&component))
        .unwrap();
    let error = runtime
        .execute(
            &prepared,
            WitToolHost::deny_all(),
            WitToolRequest::new("{}"),
        )
        .unwrap_err();

    let display = error.to_string();
    assert!(!display.contains(guest_function_name));
    assert!(!display.contains(&secret));
    match error {
        WasmError::ExecutionFailed { message, logs, .. } => {
            assert_eq!(
                message,
                wasmtime::Trap::UnreachableCodeReached.to_string(),
                "the typed trap cause should remain useful without exposing its cause chain"
            );
            assert!(!message.contains(guest_function_name));
            assert!(!message.contains(&secret));
            assert_eq!(logs.len(), 1);
            assert!(!logs[0].message.contains(&secret));
            assert!(logs[0].message.contains("operation=write"));
            assert!(logs[0].message.contains(WASM_DIAGNOSTIC_REDACTION_MARKER));
        }
        other => panic!("expected execution failure, got {other:?}"),
    }
}

#[test]
fn execution_failure_message_size_boundary_is_fail_closed() {
    let secret = format!("sk-{}", "F".repeat(24));
    let sanitized = WasmError::execution_failed(format!(
        "guest trap while writing record: {secret}; wasm-function=7"
    ));
    match &sanitized {
        WasmError::ExecutionFailed { message, .. } => {
            assert!(!message.contains(&secret));
            assert!(message.contains("wasm-function=7"));
            assert!(message.contains(WASM_DIAGNOSTIC_REDACTION_MARKER));
        }
        other => panic!("expected execution failure, got {other:?}"),
    }
    assert!(!sanitized.to_string().contains(&secret));
    assert!(sanitized.to_string().contains("wasm-function=7"));

    let retained_cause = "guest trap: unreachable; ";
    let exactly_at_limit = format!(
        "{retained_cause}{}",
        "t".repeat(WASM_DIAGNOSTIC_MAX_BYTES - retained_cause.len())
    );
    let retained = WasmError::execution_failed(exactly_at_limit.clone());
    match &retained {
        WasmError::ExecutionFailed { message, .. } => assert_eq!(message, &exactly_at_limit),
        other => panic!("expected execution failure, got {other:?}"),
    }
    assert!(retained.to_string().contains(retained_cause));

    let redacted = WasmError::execution_failed("x".repeat(WASM_DIAGNOSTIC_MAX_BYTES + 1));
    match &redacted {
        WasmError::ExecutionFailed { message, .. } => {
            assert_eq!(message, WASM_DIAGNOSTIC_REDACTION_MARKER)
        }
        other => panic!("expected execution failure, got {other:?}"),
    }
    assert_eq!(
        redacted.to_string(),
        format!("failed to execute WIT component: {WASM_DIAGNOSTIC_REDACTION_MARKER}")
    );
}

#[test]
fn guest_log_size_boundary_is_fail_closed_and_utf8_safe() {
    let exactly_at_limit = "é".repeat(WASM_DIAGNOSTIC_MAX_BYTES / 2);
    let over_limit = "x".repeat(WASM_DIAGNOSTIC_MAX_BYTES + 1);
    let straddling_secret = format!(
        "{}sk-{}",
        "p".repeat(WASM_DIAGNOSTIC_MAX_BYTES - 4),
        "S".repeat(24)
    );
    let split_codepoint = format!("{}é", "u".repeat(WASM_DIAGNOSTIC_MAX_BYTES));
    let executed = execute_diagnostic_tool(
        &[
            (0, exactly_at_limit.clone()),
            (1, over_limit),
            (2, straddling_secret.clone()),
            (3, split_codepoint),
        ],
        None,
        false,
    )
    .unwrap();

    assert_eq!(executed.logs[0].message, exactly_at_limit);
    assert_eq!(executed.logs[1].message, WASM_DIAGNOSTIC_REDACTION_MARKER);
    assert_eq!(executed.logs[2].message, WASM_DIAGNOSTIC_REDACTION_MARKER);
    assert!(!executed.logs[2].message.contains(&straddling_secret));
    assert_eq!(executed.logs[3].message, WASM_DIAGNOSTIC_REDACTION_MARKER);
    assert!(
        executed
            .logs
            .iter()
            .all(|record| record.message.is_char_boundary(record.message.len()))
    );
}

#[test]
fn diagnostic_redaction_marker_is_idempotent() {
    let executed = execute_diagnostic_tool(
        &[(2, WASM_DIAGNOSTIC_REDACTION_MARKER.to_string())],
        None,
        false,
    )
    .unwrap();

    assert_eq!(executed.logs[0].message, WASM_DIAGNOSTIC_REDACTION_MARKER);
}

#[test]
fn guest_log_count_order_levels_and_total_scan_work_are_bounded() {
    assert_eq!(
        WASM_DIAGNOSTIC_MAX_BYTES.checked_mul(WASM_DIAGNOSTIC_MAX_ENTRIES_PER_EXECUTION),
        Some(4_096_000),
        "the count and per-record caps must bound total diagnostic scan work",
    );

    let max_sized = "z".repeat(WASM_DIAGNOSTIC_MAX_BYTES);
    let logs: Vec<_> = (0..=WASM_DIAGNOSTIC_MAX_ENTRIES_PER_EXECUTION)
        .map(|index| ((index % 5) as u32, max_sized.clone()))
        .collect();
    let executed = execute_diagnostic_tool(&logs, None, false).unwrap();

    assert_eq!(
        executed.logs.len(),
        WASM_DIAGNOSTIC_MAX_ENTRIES_PER_EXECUTION
    );
    assert!(
        executed
            .logs
            .iter()
            .all(|record| record.message == max_sized)
    );
    let expected_levels = [
        ironclaw_wasm::WasmLogLevel::Trace,
        ironclaw_wasm::WasmLogLevel::Debug,
        ironclaw_wasm::WasmLogLevel::Info,
        ironclaw_wasm::WasmLogLevel::Warn,
        ironclaw_wasm::WasmLogLevel::Error,
    ];
    for (index, record) in executed.logs.iter().enumerate() {
        assert_eq!(record.level, expected_levels[index % expected_levels.len()]);
    }
}
