use std::sync::Arc;

use ironclaw_wasm::{
    DenyWasmHostHttp, RecordingWasmHostHttp, WasmError, WasmHostHttp, WasmHttpRequest,
    WasmHttpResponse, WitToolHost, WitToolRequest, WitToolRuntime, WitToolRuntimeConfig,
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
        .push_str("tool.wit", include_str!("../../../wit/tool.wit"))
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
        default_limits: ironclaw_wasm::WitToolLimits::default()
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
        default_limits: ironclaw_wasm::WitToolLimits::default()
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
        default_limits: ironclaw_wasm::WitToolLimits::default()
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
        default_limits: ironclaw_wasm::WitToolLimits::default()
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
