use std::sync::{Arc, Mutex};

use ironclaw_host_api::*;
use ironclaw_resources::*;
use ironclaw_wasm::*;
use serde_json::json;

#[test]
fn network_import_uses_host_http_with_allowlist_policy() {
    let client = RecordingHttpClient::new(WasmHttpResponse {
        status: 200,
        body: r#"{"ok":true}"#.to_string(),
    });
    let http = WasmPolicyHttpClient::new(
        client.clone(),
        NetworkPolicy {
            allowed_targets: vec![
                NetworkTargetPattern::new(Some(NetworkScheme::Https), "api.example.test", None)
                    .unwrap(),
            ],
            deny_private_ip_ranges: true,
            max_egress_bytes: Some(1024),
        },
    );
    let runtime = WasmRuntime::for_testing().unwrap();
    let module = runtime
        .prepare(http_spec("https://api.example.test/v1/echo"))
        .unwrap();
    let descriptor = make_descriptor("net-demo", "net-demo.http", RuntimeKind::Wasm);
    let reservation = sample_active_reservation();

    let result = runtime
        .invoke_json_with_network(
            &module,
            &descriptor,
            Some(&reservation),
            CapabilityInvocation { input: json!({}) },
            Arc::new(http),
        )
        .unwrap();

    assert_eq!(result.output, json!({"ok": true}));
    let requests = client.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, NetworkMethod::Get);
    assert_eq!(requests[0].url, "https://api.example.test/v1/echo");
}

#[test]
fn network_imports_deny_by_default_without_network_context() {
    let runtime = WasmRuntime::for_testing().unwrap();
    let module = runtime
        .prepare(http_spec("https://api.example.test/v1/echo"))
        .unwrap();
    let descriptor = make_descriptor("net-demo", "net-demo.http", RuntimeKind::Wasm);
    let reservation = sample_active_reservation();

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
fn network_policy_denies_unlisted_host_before_client_call() {
    let client = RecordingHttpClient::new(WasmHttpResponse {
        status: 200,
        body: r#"{"ok":true}"#.to_string(),
    });
    let http = WasmPolicyHttpClient::new(
        client.clone(),
        NetworkPolicy {
            allowed_targets: vec![
                NetworkTargetPattern::new(Some(NetworkScheme::Https), "allowed.example.test", None)
                    .unwrap(),
            ],
            deny_private_ip_ranges: true,
            max_egress_bytes: Some(1024),
        },
    );
    let runtime = WasmRuntime::for_testing().unwrap();
    let module = runtime
        .prepare(http_spec("https://blocked.example.test/v1/echo"))
        .unwrap();
    let descriptor = make_descriptor("net-demo", "net-demo.http", RuntimeKind::Wasm);
    let reservation = sample_active_reservation();

    let result = runtime
        .invoke_json_with_network(
            &module,
            &descriptor,
            Some(&reservation),
            CapabilityInvocation { input: json!({}) },
            Arc::new(http),
        )
        .unwrap();

    assert_eq!(result.output, json!({"ok": false}));
    assert!(client.requests.lock().unwrap().is_empty());
}

#[test]
fn network_policy_blocks_literal_private_ip_targets_before_client_call() {
    let client = RecordingHttpClient::new(WasmHttpResponse {
        status: 200,
        body: r#"{"ok":true}"#.to_string(),
    });
    let http = WasmPolicyHttpClient::new(
        client.clone(),
        NetworkPolicy {
            allowed_targets: vec![
                NetworkTargetPattern::new(Some(NetworkScheme::Http), "public.example.test", None)
                    .unwrap(),
            ],
            deny_private_ip_ranges: true,
            max_egress_bytes: Some(1024),
        },
    );
    let runtime = WasmRuntime::for_testing().unwrap();
    let module = runtime
        .prepare(http_spec("http://127.0.0.1/admin"))
        .unwrap();
    let descriptor = make_descriptor("net-demo", "net-demo.http", RuntimeKind::Wasm);
    let reservation = sample_active_reservation();

    let result = runtime
        .invoke_json_with_network(
            &module,
            &descriptor,
            Some(&reservation),
            CapabilityInvocation { input: json!({}) },
            Arc::new(http),
        )
        .unwrap();

    assert_eq!(result.output, json!({"ok": false}));
    assert!(client.requests.lock().unwrap().is_empty());
}

#[test]
fn network_policy_enforces_request_body_egress_limit_before_client_call() {
    let client = RecordingHttpClient::new(WasmHttpResponse {
        status: 200,
        body: r#"{"ok":true}"#.to_string(),
    });
    let http = WasmPolicyHttpClient::new(
        client.clone(),
        NetworkPolicy {
            allowed_targets: vec![
                NetworkTargetPattern::new(Some(NetworkScheme::Https), "api.example.test", None)
                    .unwrap(),
            ],
            deny_private_ip_ranges: true,
            max_egress_bytes: Some(4),
        },
    );
    let runtime = WasmRuntime::for_testing().unwrap();
    let module = runtime
        .prepare(http_post_spec(
            "https://api.example.test/v1/echo",
            "too-large",
        ))
        .unwrap();
    let descriptor = make_descriptor("net-demo", "net-demo.http", RuntimeKind::Wasm);
    let reservation = sample_active_reservation();

    let result = runtime
        .invoke_json_with_network(
            &module,
            &descriptor,
            Some(&reservation),
            CapabilityInvocation { input: json!({}) },
            Arc::new(http),
        )
        .unwrap();

    assert_eq!(result.output, json!({"ok": false}));
    assert!(client.requests.lock().unwrap().is_empty());
}

#[test]
fn network_policy_enforces_response_body_limit() {
    let client = RecordingHttpClient::new(WasmHttpResponse {
        status: 200,
        body: r#"{"ok":true,"large":"payload"}"#.to_string(),
    });
    let http = WasmPolicyHttpClient::new(
        client,
        NetworkPolicy {
            allowed_targets: vec![
                NetworkTargetPattern::new(Some(NetworkScheme::Https), "api.example.test", None)
                    .unwrap(),
            ],
            deny_private_ip_ranges: true,
            max_egress_bytes: Some(4),
        },
    );
    let runtime = WasmRuntime::for_testing().unwrap();
    let module = runtime
        .prepare(http_spec("https://api.example.test/v1/echo"))
        .unwrap();
    let descriptor = make_descriptor("net-demo", "net-demo.http", RuntimeKind::Wasm);
    let reservation = sample_active_reservation();

    let result = runtime
        .invoke_json_with_network(
            &module,
            &descriptor,
            Some(&reservation),
            CapabilityInvocation { input: json!({}) },
            Arc::new(http),
        )
        .unwrap();

    assert_eq!(result.output, json!({"ok": false}));
}

#[test]
fn network_import_bounds_response_to_guest_output_capacity() {
    let client = RecordingHttpClient::new(WasmHttpResponse {
        status: 200,
        body: format!(r#"{{"payload":"{}"}}"#, "x".repeat(600)),
    });
    let http = WasmPolicyHttpClient::new(
        client.clone(),
        NetworkPolicy {
            allowed_targets: vec![
                NetworkTargetPattern::new(Some(NetworkScheme::Https), "api.example.test", None)
                    .unwrap(),
            ],
            deny_private_ip_ranges: true,
            max_egress_bytes: None,
        },
    );
    let runtime = WasmRuntime::for_testing().unwrap();
    let module = runtime
        .prepare(http_spec("https://api.example.test/v1/echo"))
        .unwrap();
    let descriptor = make_descriptor("net-demo", "net-demo.http", RuntimeKind::Wasm);
    let reservation = sample_active_reservation();

    let result = runtime
        .invoke_json_with_network(
            &module,
            &descriptor,
            Some(&reservation),
            CapabilityInvocation { input: json!({}) },
            Arc::new(http),
        )
        .unwrap();

    assert_eq!(result.output, json!({"ok": false}));
    assert_eq!(*client.response_limits.lock().unwrap(), vec![512]);
}

#[derive(Clone)]
struct RecordingHttpClient {
    response: WasmHttpResponse,
    requests: Arc<Mutex<Vec<WasmHttpRequest>>>,
    response_limits: Arc<Mutex<Vec<usize>>>,
}

impl RecordingHttpClient {
    fn new(response: WasmHttpResponse) -> Self {
        Self {
            response,
            requests: Arc::new(Mutex::new(Vec::new())),
            response_limits: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl WasmHostHttp for RecordingHttpClient {
    fn request_utf8(
        &self,
        request: WasmHttpRequest,
        max_response_bytes: usize,
    ) -> Result<WasmHttpResponse, String> {
        self.requests.lock().unwrap().push(request);
        self.response_limits
            .lock()
            .unwrap()
            .push(max_response_bytes);
        Ok(self.response.clone())
    }
}

fn http_spec(url: &str) -> WasmModuleSpec {
    http_request_spec(url, 0, "")
}

fn http_post_spec(url: &str, body: &str) -> WasmModuleSpec {
    http_request_spec(url, 1, body)
}

fn http_request_spec(url: &str, method: i32, body: &str) -> WasmModuleSpec {
    WasmModuleSpec {
        provider: ExtensionId::new("net-demo").unwrap(),
        capability: CapabilityId::new("net-demo.http").unwrap(),
        export: "http".to_string(),
        bytes: http_module_bytes(url, method, body),
    }
}

fn http_module_bytes(url: &str, method: i32, body: &str) -> Vec<u8> {
    let url_len = url.len();
    let body_len = body.len();
    wat::parse_str(format!(
        r#"(module
            (import "host" "http_request_utf8" (func $http (param i32 i32 i32 i32 i32 i32 i32) (result i32)))
            (memory (export "memory") 1)
            (data (i32.const 64) "{url}")
            (data (i32.const 256) "{{\"ok\":false}}")
            (data (i32.const 512) "{body}")
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
            (func (export "http") (param i32 i32) (result i32)
              (local $n i32)
              i32.const {method}
              i32.const 64
              i32.const {url_len}
              i32.const 512
              i32.const {body_len}
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
            (func (export "output_ptr") (result i32) global.get $out_ptr)
            (func (export "output_len") (result i32) global.get $out_len))"#
    ))
    .unwrap()
}

fn make_descriptor(provider: &str, capability: &str, runtime: RuntimeKind) -> CapabilityDescriptor {
    CapabilityDescriptor {
        id: CapabilityId::new(capability).unwrap(),
        provider: ExtensionId::new(provider).unwrap(),
        runtime,
        trust_ceiling: TrustClass::Sandbox,
        description: "test capability".to_string(),
        parameters_schema: serde_json::json!({"type":"object"}),
        effects: vec![EffectKind::Network],
        default_permission: PermissionMode::Allow,
        resource_profile: None,
    }
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

fn sample_active_reservation() -> ActiveResourceReservation {
    let governor = InMemoryResourceGovernor::new();
    let reservation = governor
        .reserve(sample_scope(), ResourceEstimate::default())
        .unwrap();
    governor.active_reservation(reservation.id).unwrap()
}
