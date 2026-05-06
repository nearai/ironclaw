use std::sync::{Arc, Mutex};

use ironclaw_host_api::{
    CapabilityId, InvocationId, NetworkMethod, NetworkPolicy, NetworkScheme, NetworkTargetPattern,
    ProjectId, ResourceScope, RuntimeCredentialInjection, RuntimeCredentialSource,
    RuntimeCredentialTarget, RuntimeHttpEgress, RuntimeHttpEgressError, RuntimeHttpEgressRequest,
    RuntimeHttpEgressResponse, RuntimeKind, SecretHandle, TenantId, UserId,
};
use ironclaw_wasm::{
    WasmHostError, WasmHostHttp, WasmHttpRequest, WasmRuntimeCredentialProvider,
    WasmRuntimeCredentialRequest, WasmRuntimeHttpAdapter, WasmRuntimePolicyDiscarder,
    WasmStagedRuntimeCredential, WasmStagedRuntimeCredentials,
};
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
    let adapter = WasmRuntimeHttpAdapter::new(
        Arc::new(egress.clone()),
        scope.clone(),
        sample_capability_id(),
        policy.clone(),
    )
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
    assert_eq!(requests[0].capability_id, sample_capability_id());
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
    assert_eq!(requests[0].timeout_ms, Some(1234));
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
            ("cookie".to_string(), "session=sk-test-secret".to_string()),
            ("api-key".to_string(), "sk-test-secret".to_string()),
            ("x-token".to_string(), "sk-test-secret".to_string()),
            ("x-access-token".to_string(), "sk-test-secret".to_string()),
            ("x-session-token".to_string(), "sk-test-secret".to_string()),
            ("x-csrf-token".to_string(), "sk-test-secret".to_string()),
            ("x-refresh-token".to_string(), "opaque-refresh".to_string()),
            (
                "x-amz-security-token".to_string(),
                "opaque-session".to_string(),
            ),
            ("private-token".to_string(), "opaque-private".to_string()),
            ("x-credential".to_string(), "opaque-credential".to_string()),
            ("x-secret".to_string(), "sk-test-secret".to_string()),
            ("x-api-secret".to_string(), "sk-test-secret".to_string()),
            ("x-public".to_string(), "ok".to_string()),
        ],
        body: b"ok".to_vec(),
        request_bytes: 0,
        response_bytes: 2,
        redaction_applied: true,
    });
    let adapter = WasmRuntimeHttpAdapter::new(
        Arc::new(egress),
        sample_scope(),
        sample_capability_id(),
        sample_policy(),
    );

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
    assert!(!response.headers_json.contains("api-key"));
    assert!(!response.headers_json.contains("x-token"));
    assert!(!response.headers_json.contains("x-refresh-token"));
    assert!(!response.headers_json.contains("x-amz-security-token"));
    assert!(!response.headers_json.contains("private-token"));
    assert!(!response.headers_json.contains("x-credential"));
}

#[test]
fn wasm_runtime_http_adapter_combines_duplicate_response_headers() {
    let egress = RecordingRuntimeEgress::ok(RuntimeHttpEgressResponse {
        status: 200,
        headers: vec![
            (
                "link".to_string(),
                "<https://a.example>; rel=preload".to_string(),
            ),
            (
                "Link".to_string(),
                "<https://b.example>; rel=preload".to_string(),
            ),
            ("x-public".to_string(), "ok".to_string()),
        ],
        body: b"ok".to_vec(),
        request_bytes: 0,
        response_bytes: 2,
        redaction_applied: false,
    });
    let adapter = WasmRuntimeHttpAdapter::new(
        Arc::new(egress),
        sample_scope(),
        sample_capability_id(),
        sample_policy(),
    );

    let response = adapter
        .request(WasmHttpRequest {
            method: "GET".to_string(),
            url: "https://wasm-api.example.test/run".to_string(),
            headers_json: "{}".to_string(),
            body: None,
            timeout_ms: Some(1000),
        })
        .expect("response headers should encode for WASM");

    let headers = serde_json::from_str::<Value>(&response.headers_json).unwrap();
    assert_eq!(
        headers,
        json!({
            "link": "<https://a.example>; rel=preload, <https://b.example>; rel=preload",
            "x-public": "ok",
        })
    );
}

#[test]
fn wasm_runtime_http_adapter_resolves_credentials_per_request_destination() {
    let egress = RecordingRuntimeEgress::ok(RuntimeHttpEgressResponse {
        status: 200,
        headers: vec![],
        body: b"ok".to_vec(),
        request_bytes: 0,
        response_bytes: 2,
        redaction_applied: true,
    });
    let injection = sample_injection();
    let adapter = WasmRuntimeHttpAdapter::new(
        Arc::new(egress.clone()),
        sample_scope(),
        sample_capability_id(),
        sample_policy(),
    )
    .with_credential_provider(Arc::new(DestinationCredentialProvider {
        approved_url: "https://wasm-api.example.test/run".to_string(),
        injection: injection.clone(),
    }));

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
fn wasm_runtime_http_adapter_does_not_reuse_credentials_for_other_destinations() {
    let egress = RecordingRuntimeEgress::ok(RuntimeHttpEgressResponse {
        status: 200,
        headers: vec![],
        body: b"ok".to_vec(),
        request_bytes: 0,
        response_bytes: 2,
        redaction_applied: true,
    });
    let adapter = WasmRuntimeHttpAdapter::new(
        Arc::new(egress.clone()),
        sample_scope(),
        sample_capability_id(),
        multi_target_policy(),
    )
    .with_credential_provider(Arc::new(DestinationCredentialProvider {
        approved_url: "https://wasm-api.example.test/run".to_string(),
        injection: sample_injection(),
    }));

    adapter
        .request(WasmHttpRequest {
            method: "GET".to_string(),
            url: "https://other-api.example.test/run".to_string(),
            headers_json: "{}".to_string(),
            body: None,
            timeout_ms: Some(1000),
        })
        .unwrap();

    let requests = egress.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].credential_injections.is_empty());
}

#[test]
fn wasm_runtime_http_adapter_can_build_staged_obligation_credentials() {
    let egress = RecordingRuntimeEgress::ok(RuntimeHttpEgressResponse {
        status: 200,
        headers: vec![],
        body: b"ok".to_vec(),
        request_bytes: 0,
        response_bytes: 2,
        redaction_applied: true,
    });
    let capability_id = sample_capability_id();
    let handle = SecretHandle::new("api-token").unwrap();
    let target = RuntimeCredentialTarget::Header {
        name: "authorization".to_string(),
        prefix: Some("Bearer ".to_string()),
    };
    let adapter = WasmRuntimeHttpAdapter::new(
        Arc::new(egress.clone()),
        sample_scope(),
        capability_id.clone(),
        multi_target_policy(),
    )
    .with_credential_provider(Arc::new(WasmStagedRuntimeCredentials::new(vec![
        WasmStagedRuntimeCredential::for_exact_url(
            handle.clone(),
            target.clone(),
            true,
            "https://wasm-api.example.test/run".to_string(),
        ),
    ])));

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
    assert_eq!(
        requests[0].credential_injections,
        vec![RuntimeCredentialInjection {
            handle,
            source: RuntimeCredentialSource::StagedObligation { capability_id },
            target,
            required: true,
        }]
    );
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
    let adapter = WasmRuntimeHttpAdapter::new(
        Arc::new(egress.clone()),
        sample_scope(),
        sample_capability_id(),
        sample_policy(),
    );

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
        sample_capability_id(),
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
        sample_capability_id(),
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
        sample_capability_id(),
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
        sample_capability_id(),
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

#[test]
fn wasm_runtime_http_adapter_marks_zero_body_response_failures_after_send() {
    let adapter = WasmRuntimeHttpAdapter::new(
        Arc::new(RecordingRuntimeEgress::err(
            RuntimeHttpEgressError::Network {
                reason: "response body limit exceeded for sk-test-secret".to_string(),
                request_bytes: 0,
                response_bytes: 6,
            },
        )),
        sample_scope(),
        sample_capability_id(),
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
        .expect_err("zero-body response-stage failures should still be post-send failures");

    let rendered = error.to_string();
    assert!(matches!(
        error,
        WasmHostError::FailedAfterRequestSent(reason) if reason.contains("network_error")
    ));
    assert!(!rendered.contains("sk-test-secret"));
}

#[test]
fn wasm_runtime_http_adapter_redacts_credential_provider_errors() {
    let adapter = WasmRuntimeHttpAdapter::new(
        Arc::new(RecordingRuntimeEgress::ok(RuntimeHttpEgressResponse {
            status: 200,
            headers: vec![],
            body: b"ok".to_vec(),
            request_bytes: 0,
            response_bytes: 2,
            redaction_applied: false,
        })),
        sample_scope(),
        sample_capability_id(),
        sample_policy(),
    )
    .with_credential_provider(Arc::new(FailingCredentialProvider));

    let error = adapter
        .request(WasmHttpRequest {
            method: "GET".to_string(),
            url: "https://wasm-api.example.test/run".to_string(),
            headers_json: "{}".to_string(),
            body: None,
            timeout_ms: Some(1000),
        })
        .expect_err("credential provider errors should be sanitized before WASM visibility");

    let rendered = error.to_string();
    assert!(matches!(error, WasmHostError::Unavailable(_)));
    assert!(rendered.contains("credential_unavailable"));
    assert!(!rendered.contains("gmail-token"));
    assert!(!rendered.contains("sk-test-secret"));
}

#[test]
fn wasm_runtime_http_adapter_discards_staged_policy_on_pre_egress_request_failure() {
    let egress = RecordingRuntimeEgress::ok(RuntimeHttpEgressResponse {
        status: 200,
        headers: vec![],
        body: b"ok".to_vec(),
        request_bytes: 0,
        response_bytes: 2,
        redaction_applied: false,
    });
    let discarder = Arc::new(RecordingPolicyDiscarder::default());
    let scope = sample_scope();
    let capability_id = sample_capability_id();
    let adapter = WasmRuntimeHttpAdapter::new(
        Arc::new(egress.clone()),
        scope.clone(),
        capability_id.clone(),
        sample_policy(),
    )
    .with_policy_discarder(discarder.clone());

    let error = adapter
        .request(WasmHttpRequest {
            method: "TRACE".to_string(),
            url: "https://wasm-api.example.test/run".to_string(),
            headers_json: "{}".to_string(),
            body: None,
            timeout_ms: Some(1000),
        })
        .expect_err("unsupported methods should fail before shared egress");

    assert!(matches!(error, WasmHostError::Denied(_)));
    assert!(egress.requests.lock().unwrap().is_empty());
    assert_eq!(discarder.discards(), vec![(scope, capability_id)]);
}

#[test]
fn wasm_runtime_http_adapter_discards_staged_policy_on_credential_provider_failure() {
    let egress = RecordingRuntimeEgress::ok(RuntimeHttpEgressResponse {
        status: 200,
        headers: vec![],
        body: b"ok".to_vec(),
        request_bytes: 0,
        response_bytes: 2,
        redaction_applied: false,
    });
    let discarder = Arc::new(RecordingPolicyDiscarder::default());
    let scope = sample_scope();
    let capability_id = sample_capability_id();
    let adapter = WasmRuntimeHttpAdapter::new(
        Arc::new(egress.clone()),
        scope.clone(),
        capability_id.clone(),
        sample_policy(),
    )
    .with_policy_discarder(discarder.clone())
    .with_credential_provider(Arc::new(FailingCredentialProvider));

    let error = adapter
        .request(WasmHttpRequest {
            method: "GET".to_string(),
            url: "https://wasm-api.example.test/run".to_string(),
            headers_json: "{}".to_string(),
            body: None,
            timeout_ms: Some(1000),
        })
        .expect_err("credential provider failures should fail before shared egress");

    assert!(matches!(error, WasmHostError::Unavailable(_)));
    assert!(egress.requests.lock().unwrap().is_empty());
    assert_eq!(discarder.discards(), vec![(scope, capability_id)]);
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

#[derive(Debug, Default)]
struct RecordingPolicyDiscarder {
    discards: Mutex<Vec<(ResourceScope, CapabilityId)>>,
}

impl RecordingPolicyDiscarder {
    fn discards(&self) -> Vec<(ResourceScope, CapabilityId)> {
        self.discards.lock().unwrap().clone()
    }
}

impl WasmRuntimePolicyDiscarder for RecordingPolicyDiscarder {
    fn discard(&self, scope: &ResourceScope, capability_id: &CapabilityId) {
        self.discards
            .lock()
            .unwrap()
            .push((scope.clone(), capability_id.clone()));
    }
}

#[derive(Debug)]
struct DestinationCredentialProvider {
    approved_url: String,
    injection: RuntimeCredentialInjection,
}

impl WasmRuntimeCredentialProvider for DestinationCredentialProvider {
    fn credential_injections(
        &self,
        request: &WasmRuntimeCredentialRequest,
    ) -> Result<Vec<RuntimeCredentialInjection>, WasmHostError> {
        let WasmRuntimeCredentialRequest {
            scope: _,
            capability_id: _,
            method: _,
            url,
            headers: _,
        } = request;
        if url == &self.approved_url {
            Ok(vec![self.injection.clone()])
        } else {
            Ok(Vec::new())
        }
    }
}

#[derive(Debug)]
struct FailingCredentialProvider;

impl WasmRuntimeCredentialProvider for FailingCredentialProvider {
    fn credential_injections(
        &self,
        _request: &WasmRuntimeCredentialRequest,
    ) -> Result<Vec<RuntimeCredentialInjection>, WasmHostError> {
        Err(WasmHostError::Unavailable(
            "gmail-token unavailable: sk-test-secret".to_string(),
        ))
    }
}

fn sample_capability_id() -> CapabilityId {
    CapabilityId::new("wasm.http").unwrap()
}

fn sample_injection() -> RuntimeCredentialInjection {
    RuntimeCredentialInjection {
        handle: SecretHandle::new("api-token").unwrap(),
        source: RuntimeCredentialSource::SecretStoreLease,
        target: RuntimeCredentialTarget::Header {
            name: "authorization".to_string(),
            prefix: Some("Bearer ".to_string()),
        },
        required: true,
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

fn multi_target_policy() -> NetworkPolicy {
    NetworkPolicy {
        allowed_targets: vec![
            NetworkTargetPattern {
                scheme: Some(NetworkScheme::Https),
                host_pattern: "wasm-api.example.test".to_string(),
                port: None,
            },
            NetworkTargetPattern {
                scheme: Some(NetworkScheme::Https),
                host_pattern: "other-api.example.test".to_string(),
                port: None,
            },
        ],
        deny_private_ip_ranges: true,
        max_egress_bytes: Some(4096),
    }
}
