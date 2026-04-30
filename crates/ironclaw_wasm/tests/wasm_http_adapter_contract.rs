use std::sync::{Arc, Mutex};

use ironclaw_host_api::{
    InvocationId, NetworkMethod, NetworkPolicy, NetworkScheme, NetworkTargetPattern, ProjectId,
    ResourceScope, RuntimeCredentialInjection, RuntimeCredentialTarget, RuntimeHttpEgress,
    RuntimeHttpEgressError, RuntimeHttpEgressRequest, RuntimeHttpEgressResponse, RuntimeKind,
    SecretHandle, TenantId, UserId,
};
use ironclaw_wasm::{WasmHostError, WasmHostHttp, WasmHttpRequest, WasmRuntimeHttpAdapter};
use serde_json::{Value, json};

#[test]
fn wasm_runtime_http_adapter_uses_shared_runtime_egress() {
    let egress = RecordingRuntimeEgress::ok(RuntimeHttpEgressResponse {
        status: 201,
        headers: vec![("content-type".to_string(), "application/json".to_string())],
        body: br#"{"ok":true}"#.to_vec(),
        request_bytes: 7,
        response_bytes: 11,
        redaction_applied: false,
    });
    let scope = sample_scope();
    let policy = sample_policy();
    let adapter =
        WasmRuntimeHttpAdapter::new(Arc::new(egress.clone()), scope.clone(), policy.clone())
            .with_response_body_limit(Some(4096));

    let response = adapter
        .request(WasmHttpRequest {
            method: "POST".to_string(),
            url: "https://wasm-api.example.test/run".to_string(),
            headers_json: r#"{"content-type":"application/json","x-trace":"abc"}"#.to_string(),
            body: Some(br#"{"a":1}"#.to_vec()),
            timeout_ms: Some(1234),
        })
        .expect("WASM host HTTP should delegate to shared runtime egress");

    assert_eq!(response.status, 201);
    assert_eq!(response.body, br#"{"ok":true}"#);
    let response_headers = serde_json::from_str::<Value>(&response.headers_json).unwrap();
    assert_eq!(
        response_headers,
        json!({"content-type": "application/json"})
    );

    let requests = egress.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].runtime, RuntimeKind::Wasm);
    assert_eq!(requests[0].scope, scope);
    assert_eq!(requests[0].method, NetworkMethod::Post);
    assert_eq!(requests[0].url, "https://wasm-api.example.test/run");
    assert_eq!(
        requests[0].headers,
        vec![
            ("content-type".to_string(), "application/json".to_string()),
            ("x-trace".to_string(), "abc".to_string()),
        ]
    );
    assert_eq!(requests[0].body, br#"{"a":1}"#);
    assert_eq!(requests[0].network_policy, policy);
    assert!(requests[0].credential_injections.is_empty());
    assert_eq!(requests[0].response_body_limit, Some(4096));
}

#[test]
fn wasm_runtime_http_adapter_strips_sensitive_response_headers() {
    let egress = RecordingRuntimeEgress::ok(RuntimeHttpEgressResponse {
        status: 200,
        headers: vec![
            (
                "authorization".to_string(),
                "Bearer sk-test-secret".to_string(),
            ),
            (
                "set-cookie".to_string(),
                "session=sk-test-secret".to_string(),
            ),
            ("x-public".to_string(), "ok".to_string()),
        ],
        body: b"ok".to_vec(),
        request_bytes: 0,
        response_bytes: 2,
        redaction_applied: true,
    });
    let adapter = WasmRuntimeHttpAdapter::new(Arc::new(egress), sample_scope(), sample_policy());

    let response = adapter
        .request(WasmHttpRequest {
            method: "GET".to_string(),
            url: "https://wasm-api.example.test/run".to_string(),
            headers_json: "{}".to_string(),
            body: None,
            timeout_ms: Some(1000),
        })
        .expect("sanitized response should reach WASM");

    let headers = serde_json::from_str::<Value>(&response.headers_json).unwrap();
    assert_eq!(headers, json!({"x-public": "ok"}));
    assert!(!response.headers_json.contains("sk-test-secret"));
    assert!(!response.headers_json.contains("authorization"));
    assert!(!response.headers_json.contains("set-cookie"));
}

#[test]
fn wasm_runtime_http_adapter_forwards_host_approved_credential_injections() {
    let egress = RecordingRuntimeEgress::ok(RuntimeHttpEgressResponse {
        status: 200,
        headers: vec![],
        body: b"ok".to_vec(),
        request_bytes: 0,
        response_bytes: 2,
        redaction_applied: true,
    });
    let injection = RuntimeCredentialInjection {
        handle: SecretHandle::new("api-token").unwrap(),
        target: RuntimeCredentialTarget::Header {
            name: "authorization".to_string(),
            prefix: Some("Bearer ".to_string()),
        },
        required: true,
    };
    let adapter =
        WasmRuntimeHttpAdapter::new(Arc::new(egress.clone()), sample_scope(), sample_policy())
            .with_credential_injections(vec![injection.clone()]);

    adapter
        .request(WasmHttpRequest {
            method: "GET".to_string(),
            url: "https://wasm-api.example.test/run".to_string(),
            headers_json: "{}".to_string(),
            body: None,
            timeout_ms: Some(1000),
        })
        .unwrap();

    let requests = egress.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].credential_injections, vec![injection]);
}

#[test]
fn wasm_runtime_http_adapter_rejects_invalid_guest_headers_before_egress() {
    let egress = RecordingRuntimeEgress::ok(RuntimeHttpEgressResponse {
        status: 200,
        headers: vec![],
        body: vec![],
        request_bytes: 0,
        response_bytes: 0,
        redaction_applied: false,
    });
    let adapter =
        WasmRuntimeHttpAdapter::new(Arc::new(egress.clone()), sample_scope(), sample_policy());

    let error = adapter
        .request(WasmHttpRequest {
            method: "POST".to_string(),
            url: "https://wasm-api.example.test/run".to_string(),
            headers_json: r#"{"x-number": 1}"#.to_string(),
            body: Some(b"body".to_vec()),
            timeout_ms: Some(1000),
        })
        .expect_err("non-string header values should fail before shared egress");

    assert!(matches!(error, WasmHostError::Denied(_)));
    assert!(egress.requests.lock().unwrap().is_empty());
}

#[test]
fn wasm_runtime_http_adapter_redacts_credential_errors_before_guest_visibility() {
    let adapter = WasmRuntimeHttpAdapter::new(
        Arc::new(RecordingRuntimeEgress::err(
            RuntimeHttpEgressError::Credential {
                reason: "secret handle gmail-token unavailable: sk-test-secret".to_string(),
            },
        )),
        sample_scope(),
        sample_policy(),
    );

    let error = adapter
        .request(WasmHttpRequest {
            method: "GET".to_string(),
            url: "https://wasm-api.example.test/run".to_string(),
            headers_json: "{}".to_string(),
            body: None,
            timeout_ms: Some(1000),
        })
        .expect_err("credential failures should not expose secret metadata to WASM");

    let rendered = error.to_string();
    assert!(matches!(error, WasmHostError::Unavailable(_)));
    assert!(rendered.contains("credential_unavailable"));
    assert!(!rendered.contains("gmail-token"));
    assert!(!rendered.contains("sk-test-secret"));
}

#[test]
fn wasm_runtime_http_adapter_redacts_shared_request_error_reasons() {
    let adapter = WasmRuntimeHttpAdapter::new(
        Arc::new(RecordingRuntimeEgress::err(
            RuntimeHttpEgressError::Request {
                reason: "sensitive_header_denied:authorization Bearer sk-test-secret".to_string(),
                request_bytes: 0,
                response_bytes: 0,
            },
        )),
        sample_scope(),
        sample_policy(),
    );

    let error = adapter
        .request(WasmHttpRequest {
            method: "POST".to_string(),
            url: "https://wasm-api.example.test/run".to_string(),
            headers_json: "{}".to_string(),
            body: Some(br#"{"a":1}"#.to_vec()),
            timeout_ms: Some(1000),
        })
        .expect_err("request errors should be stable and sanitized at the WIT boundary");

    let rendered = error.to_string();
    assert!(matches!(error, WasmHostError::Denied(_)));
    assert!(rendered.contains("request_denied"));
    assert!(!rendered.contains("authorization"));
    assert!(!rendered.contains("sk-test-secret"));
}

#[test]
fn wasm_runtime_http_adapter_redacts_shared_network_denial_reasons() {
    let adapter = WasmRuntimeHttpAdapter::new(
        Arc::new(RecordingRuntimeEgress::err(
            RuntimeHttpEgressError::Network {
                reason: "private target 10.0.0.7 denied for secret sk-test-secret".to_string(),
                request_bytes: 0,
                response_bytes: 0,
            },
        )),
        sample_scope(),
        sample_policy(),
    );

    let error = adapter
        .request(WasmHttpRequest {
            method: "POST".to_string(),
            url: "https://wasm-api.example.test/run".to_string(),
            headers_json: "{}".to_string(),
            body: Some(br#"{"a":1}"#.to_vec()),
            timeout_ms: Some(1000),
        })
        .expect_err("network denials should be stable and sanitized at the WIT boundary");

    let rendered = error.to_string();
    assert!(matches!(error, WasmHostError::Denied(_)));
    assert!(rendered.contains("network_error"));
    assert!(!rendered.contains("10.0.0.7"));
    assert!(!rendered.contains("sk-test-secret"));
}

#[test]
fn wasm_runtime_http_adapter_marks_post_send_shared_egress_errors_for_accounting() {
    let adapter = WasmRuntimeHttpAdapter::new(
        Arc::new(RecordingRuntimeEgress::err(
            RuntimeHttpEgressError::Response {
                reason: "response leaked secret sk-test-secret".to_string(),
                request_bytes: 7,
                response_bytes: 43,
            },
        )),
        sample_scope(),
        sample_policy(),
    );

    let error = adapter
        .request(WasmHttpRequest {
            method: "POST".to_string(),
            url: "https://wasm-api.example.test/run".to_string(),
            headers_json: "{}".to_string(),
            body: Some(br#"{"a":1}"#.to_vec()),
            timeout_ms: Some(1000),
        })
        .expect_err("post-send response errors should preserve request accounting");

    let rendered = error.to_string();
    assert!(matches!(
        error,
        WasmHostError::FailedAfterRequestSent(reason) if reason.contains("response_error")
    ));
    assert!(!rendered.contains("sk-test-secret"));
}

#[derive(Clone)]
struct RecordingRuntimeEgress {
    response: Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError>,
    requests: Arc<Mutex<Vec<RuntimeHttpEgressRequest>>>,
}

impl RecordingRuntimeEgress {
    fn ok(response: RuntimeHttpEgressResponse) -> Self {
        Self {
            response: Ok(response),
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn err(error: RuntimeHttpEgressError) -> Self {
        Self {
            response: Err(error),
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl RuntimeHttpEgress for RecordingRuntimeEgress {
    fn execute(
        &self,
        request: RuntimeHttpEgressRequest,
    ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
        self.requests.lock().unwrap().push(request);
        self.response.clone()
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

fn sample_policy() -> NetworkPolicy {
    NetworkPolicy {
        allowed_targets: vec![NetworkTargetPattern {
            scheme: Some(NetworkScheme::Https),
            host_pattern: "wasm-api.example.test".to_string(),
            port: None,
        }],
        deny_private_ip_ranges: true,
        max_egress_bytes: Some(4096),
    }
}
