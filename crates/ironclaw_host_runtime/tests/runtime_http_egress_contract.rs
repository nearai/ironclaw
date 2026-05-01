use ironclaw_host_api::{
    InvocationId, NetworkMethod, NetworkPolicy, NetworkScheme, NetworkTargetPattern, ResourceScope,
    RuntimeCredentialInjection, RuntimeCredentialTarget, RuntimeHttpEgress,
    RuntimeHttpEgressRequest, RuntimeKind, SecretHandle, TenantId, UserId,
};
use ironclaw_host_runtime::HostHttpEgressService;
use ironclaw_network::{
    NetworkHttpEgress, NetworkHttpError, NetworkHttpRequest, NetworkHttpResponse, NetworkUsage,
};
use ironclaw_secrets::{
    InMemorySecretStore, SecretLease, SecretLeaseId, SecretMaterial, SecretMetadata, SecretStore,
    SecretStoreError,
};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

#[test]
fn host_http_egress_injects_leased_credentials_and_redacts_errors() {
    let network = RecordingNetwork::err(NetworkHttpError::Transport {
        reason: "upstream rejected token sk-test-secret".to_string(),
        request_bytes: 12,
        response_bytes: 0,
    });
    let network_recorder = network.requests.clone();
    let secrets = InMemorySecretStore::new();
    let scope = sample_scope();
    let handle = SecretHandle::new("api-token").unwrap();
    block_on_test(secrets.put(
        scope.clone(),
        handle.clone(),
        SecretMaterial::from("sk-test-secret"),
    ))
    .unwrap();
    let service = HostHttpEgressService::new(network, secrets);

    let error = service
        .execute(RuntimeHttpEgressRequest {
            runtime: RuntimeKind::Script,
            scope,
            method: NetworkMethod::Post,
            url: "https://api.example.test/v1/run".to_string(),
            headers: vec![],
            body: b"hello".to_vec(),
            network_policy: sample_policy(),
            credential_injections: vec![RuntimeCredentialInjection {
                handle,
                target: RuntimeCredentialTarget::Header {
                    name: "authorization".to_string(),
                    prefix: Some("Bearer ".to_string()),
                },
                required: true,
            }],
            response_body_limit: Some(4096),
            timeout_ms: None,
        })
        .expect_err("network error should be sanitized");

    let rendered = error.to_string();
    assert!(rendered.contains("transport_failed"));
    assert!(!rendered.contains("sk-test-secret"));
    assert_eq!(error.request_bytes(), 12);
    let requests = network_recorder.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0]
            .headers
            .iter()
            .find(|(name, _)| name == "authorization"),
        Some(&(
            "authorization".to_string(),
            "Bearer sk-test-secret".to_string()
        ))
    );
}

#[test]
fn host_http_egress_requires_available_required_credentials_before_network() {
    let network = RecordingNetwork::ok(NetworkHttpResponse {
        status: 200,
        headers: vec![],
        body: br#"{"ok":true}"#.to_vec(),
        usage: NetworkUsage {
            request_bytes: 5,
            response_bytes: 11,
            resolved_ip: None,
        },
    });
    let network_recorder = network.requests.clone();
    let service = HostHttpEgressService::new(network, InMemorySecretStore::new());

    let error = service
        .execute(RuntimeHttpEgressRequest {
            runtime: RuntimeKind::Script,
            scope: sample_scope(),
            method: NetworkMethod::Post,
            url: "https://api.example.test/v1/run".to_string(),
            headers: vec![],
            body: b"hello".to_vec(),
            network_policy: sample_policy(),
            credential_injections: vec![RuntimeCredentialInjection {
                handle: SecretHandle::new("missing-token").unwrap(),
                target: RuntimeCredentialTarget::Header {
                    name: "authorization".to_string(),
                    prefix: Some("Bearer ".to_string()),
                },
                required: true,
            }],
            response_body_limit: Some(4096),
            timeout_ms: None,
        })
        .expect_err("missing required credentials should fail before network dispatch");

    assert!(matches!(
        error,
        ironclaw_host_api::RuntimeHttpEgressError::Credential { .. }
    ));
    assert!(network_recorder.lock().unwrap().is_empty());
}

#[test]
fn host_http_egress_injects_and_redacts_url_encoded_query_credentials() {
    let network = UrlEchoNetwork::new();
    let network_recorder = network.requests.clone();
    let secrets = InMemorySecretStore::new();
    let scope = sample_scope();
    let handle = SecretHandle::new("api-token").unwrap();
    block_on_test(secrets.put(
        scope.clone(),
        handle.clone(),
        SecretMaterial::from("secret with/slash+plus?"),
    ))
    .unwrap();
    let service = HostHttpEgressService::new(network, secrets);

    let error = service
        .execute(RuntimeHttpEgressRequest {
            runtime: RuntimeKind::Script,
            scope,
            method: NetworkMethod::Post,
            url: "https://api.example.test/v1/run".to_string(),
            headers: vec![],
            body: b"hello".to_vec(),
            network_policy: sample_policy(),
            credential_injections: vec![RuntimeCredentialInjection {
                handle,
                target: RuntimeCredentialTarget::QueryParam {
                    name: "token".to_string(),
                },
                required: true,
            }],
            response_body_limit: Some(4096),
            timeout_ms: None,
        })
        .expect_err("network error should be sanitized");

    let rendered = error.to_string();
    assert!(rendered.contains("transport_failed"));
    assert!(!rendered.contains("secret with/slash+plus?"));
    assert!(!rendered.contains("secret+with%2Fslash%2Bplus%3F"));
    let requests = network_recorder.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].url,
        "https://api.example.test/v1/run?token=secret+with%2Fslash%2Bplus%3F"
    );
}

#[test]
fn host_http_egress_forwards_timeout_to_network() {
    let network = RecordingNetwork::ok(NetworkHttpResponse {
        status: 200,
        headers: vec![],
        body: br#"{\"ok\":true}"#.to_vec(),
        usage: NetworkUsage {
            request_bytes: 5,
            response_bytes: 11,
            resolved_ip: None,
        },
    });
    let network_recorder = network.requests.clone();
    let service = HostHttpEgressService::new(network, InMemorySecretStore::new());

    service
        .execute(RuntimeHttpEgressRequest {
            runtime: RuntimeKind::Wasm,
            scope: sample_scope(),
            method: NetworkMethod::Post,
            url: "https://api.example.test/v1/run".to_string(),
            headers: vec![],
            body: b"hello".to_vec(),
            network_policy: sample_policy(),
            credential_injections: vec![],
            response_body_limit: Some(4096),
            timeout_ms: Some(250),
        })
        .expect("network response should be returned");

    let requests = network_recorder.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].timeout_ms, Some(250));
}

#[test]
fn host_http_egress_preserves_request_and_response_byte_accounting() {
    let network = RecordingNetwork::ok(NetworkHttpResponse {
        status: 200,
        headers: vec![],
        body: br#"{"ok":true}"#.to_vec(),
        usage: NetworkUsage {
            request_bytes: 5,
            response_bytes: 11,
            resolved_ip: None,
        },
    });
    let network_recorder = network.requests.clone();
    let service = HostHttpEgressService::new(network, InMemorySecretStore::new());

    let response = service
        .execute(RuntimeHttpEgressRequest {
            runtime: RuntimeKind::Mcp,
            scope: sample_scope(),
            method: NetworkMethod::Post,
            url: "https://api.example.test/mcp".to_string(),
            headers: vec![],
            body: b"hello".to_vec(),
            network_policy: sample_policy(),
            credential_injections: vec![],
            response_body_limit: Some(4096),
            timeout_ms: None,
        })
        .expect("network response should be returned");

    assert_eq!(response.request_bytes, 5);
    assert_eq!(response.response_bytes, 11);
    let requests = network_recorder.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].body, b"hello");
    assert_eq!(requests[0].response_body_limit, Some(4096));
}

#[test]
fn host_http_egress_redacts_injected_credentials_from_runtime_visible_response() {
    let network = RecordingNetwork::ok(NetworkHttpResponse {
        status: 200,
        headers: vec![
            (
                "set-cookie".to_string(),
                "session=sk-test-secret".to_string(),
            ),
            ("x-echo".to_string(), "sk-test-secret".to_string()),
        ],
        body: b"upstream echoed sk-test-secret".to_vec(),
        usage: NetworkUsage {
            request_bytes: 5,
            response_bytes: 29,
            resolved_ip: None,
        },
    });
    let secrets = InMemorySecretStore::new();
    let scope = sample_scope();
    let handle = SecretHandle::new("api-token").unwrap();
    block_on_test(secrets.put(
        scope.clone(),
        handle.clone(),
        SecretMaterial::from("sk-test-secret"),
    ))
    .unwrap();
    let service = HostHttpEgressService::new(network, secrets);

    let response = service
        .execute(RuntimeHttpEgressRequest {
            runtime: RuntimeKind::Script,
            scope,
            method: NetworkMethod::Post,
            url: "https://api.example.test/v1/run".to_string(),
            headers: vec![],
            body: b"hello".to_vec(),
            network_policy: sample_policy(),
            credential_injections: vec![RuntimeCredentialInjection {
                handle,
                target: RuntimeCredentialTarget::Header {
                    name: "authorization".to_string(),
                    prefix: Some("Bearer ".to_string()),
                },
                required: true,
            }],
            response_body_limit: Some(4096),
            timeout_ms: None,
        })
        .expect("sanitized response should be returned");

    assert!(response.redaction_applied);
    assert_eq!(
        response.headers,
        vec![("x-echo".to_string(), "[REDACTED]".to_string())]
    );
    assert_eq!(response.body, b"upstream echoed [REDACTED]".to_vec());
}

#[test]
fn host_http_egress_redacts_lowercase_percent_encoded_secret_echoes() {
    let network = RecordingNetwork::ok(NetworkHttpResponse {
        status: 200,
        headers: vec![(
            "x-echo".to_string(),
            "secret+with%2fslash%2bplus%3f".to_string(),
        )],
        body: b"upstream echoed secret+with%2fslash%2bplus%3f".to_vec(),
        usage: NetworkUsage {
            request_bytes: 5,
            response_bytes: 45,
            resolved_ip: None,
        },
    });
    let secrets = InMemorySecretStore::new();
    let scope = sample_scope();
    let handle = SecretHandle::new("api-token").unwrap();
    block_on_test(secrets.put(
        scope.clone(),
        handle.clone(),
        SecretMaterial::from("secret with/slash+plus?"),
    ))
    .unwrap();
    let service = HostHttpEgressService::new(network, secrets);

    let response = service
        .execute(RuntimeHttpEgressRequest {
            runtime: RuntimeKind::Script,
            scope,
            method: NetworkMethod::Post,
            url: "https://api.example.test/v1/run".to_string(),
            headers: vec![],
            body: b"hello".to_vec(),
            network_policy: sample_policy(),
            credential_injections: vec![RuntimeCredentialInjection {
                handle,
                target: RuntimeCredentialTarget::QueryParam {
                    name: "token".to_string(),
                },
                required: true,
            }],
            response_body_limit: Some(4096),
            timeout_ms: None,
        })
        .expect("lowercase percent-encoded echoed credentials should be redacted");

    assert!(response.redaction_applied);
    assert_eq!(
        response.headers,
        vec![("x-echo".to_string(), "[REDACTED]".to_string())]
    );
    assert_eq!(response.body, b"upstream echoed [REDACTED]".to_vec());
}

#[test]
fn host_http_egress_strips_all_sensitive_response_headers() {
    let network = RecordingNetwork::ok(NetworkHttpResponse {
        status: 200,
        headers: vec![
            ("api-key".to_string(), "short-manual-key".to_string()),
            ("x-token".to_string(), "short-manual-key".to_string()),
            ("x-access-token".to_string(), "short-manual-key".to_string()),
            (
                "x-session-token".to_string(),
                "short-manual-key".to_string(),
            ),
            ("x-csrf-token".to_string(), "short-manual-key".to_string()),
            ("x-refresh-token".to_string(), "opaque-refresh".to_string()),
            (
                "x-amz-security-token".to_string(),
                "opaque-session".to_string(),
            ),
            ("private-token".to_string(), "opaque-private".to_string()),
            ("x-credential".to_string(), "opaque-credential".to_string()),
            ("x-secret".to_string(), "short-manual-key".to_string()),
            ("x-api-secret".to_string(), "short-manual-key".to_string()),
            ("x-public".to_string(), "ok".to_string()),
        ],
        body: b"{}".to_vec(),
        usage: NetworkUsage {
            request_bytes: 5,
            response_bytes: 2,
            resolved_ip: None,
        },
    });
    let service = HostHttpEgressService::new(network, InMemorySecretStore::new());

    let response = service
        .execute(RuntimeHttpEgressRequest {
            runtime: RuntimeKind::Script,
            scope: sample_scope(),
            method: NetworkMethod::Post,
            url: "https://api.example.test/v1/run".to_string(),
            headers: vec![],
            body: b"hello".to_vec(),
            network_policy: sample_policy(),
            credential_injections: vec![],
            response_body_limit: Some(4096),
            timeout_ms: None,
        })
        .expect("sensitive response headers should be stripped before runtime visibility");

    assert!(response.redaction_applied);
    assert_eq!(
        response.headers,
        vec![("x-public".to_string(), "ok".to_string())]
    );
}

#[test]
fn host_http_egress_blocks_credential_shaped_response_body() {
    let network = RecordingNetwork::ok(NetworkHttpResponse {
        status: 200,
        headers: vec![],
        body: b"leaked key sk-proj-test1234567890abcdefghij".to_vec(),
        usage: NetworkUsage {
            request_bytes: 5,
            response_bytes: 43,
            resolved_ip: None,
        },
    });
    let service = HostHttpEgressService::new(network, InMemorySecretStore::new());

    let error = service
        .execute(RuntimeHttpEgressRequest {
            runtime: RuntimeKind::Script,
            scope: sample_scope(),
            method: NetworkMethod::Post,
            url: "https://api.example.test/v1/run".to_string(),
            headers: vec![],
            body: b"hello".to_vec(),
            network_policy: sample_policy(),
            credential_injections: vec![],
            response_body_limit: Some(4096),
            timeout_ms: None,
        })
        .expect_err("credential-shaped response bodies should not reach runtimes");

    assert!(matches!(
        error,
        ironclaw_host_api::RuntimeHttpEgressError::Response { .. }
    ));
    assert!(!error.to_string().contains("sk-proj-test"));
    assert_eq!(error.request_bytes(), 5);
    assert_eq!(error.response_bytes(), 43);
}

#[test]
fn host_http_egress_blocks_credential_shaped_runtime_request_before_network() {
    let network = RecordingNetwork::ok(NetworkHttpResponse {
        status: 200,
        headers: vec![],
        body: br#"{"ok":true}"#.to_vec(),
        usage: NetworkUsage {
            request_bytes: 5,
            response_bytes: 11,
            resolved_ip: None,
        },
    });
    let network_recorder = network.requests.clone();
    let service = HostHttpEgressService::new(network, InMemorySecretStore::new());

    let error = service
        .execute(RuntimeHttpEgressRequest {
            runtime: RuntimeKind::Script,
            scope: sample_scope(),
            method: NetworkMethod::Post,
            url: "https://api.example.test/v1/run".to_string(),
            headers: vec![],
            body: b"sk-proj-test1234567890abcdefghij".to_vec(),
            network_policy: sample_policy(),
            credential_injections: vec![],
            response_body_limit: Some(4096),
            timeout_ms: None,
        })
        .expect_err("credential-shaped runtime requests should fail before network dispatch");

    assert!(matches!(
        error,
        ironclaw_host_api::RuntimeHttpEgressError::Request { .. }
    ));
    assert!(!error.to_string().contains("sk-proj-test"));
    assert!(network_recorder.lock().unwrap().is_empty());
}

#[test]
fn host_http_egress_blocks_runtime_supplied_sensitive_headers_before_network() {
    let network = RecordingNetwork::ok(NetworkHttpResponse {
        status: 200,
        headers: vec![],
        body: br#"{"ok":true}"#.to_vec(),
        usage: NetworkUsage {
            request_bytes: 5,
            response_bytes: 11,
            resolved_ip: None,
        },
    });
    let network_recorder = network.requests.clone();
    let service = HostHttpEgressService::new(network, InMemorySecretStore::new());

    let error = service
        .execute(RuntimeHttpEgressRequest {
            runtime: RuntimeKind::Script,
            scope: sample_scope(),
            method: NetworkMethod::Post,
            url: "https://api.example.test/v1/run".to_string(),
            headers: vec![(
                "Authorization".to_string(),
                "Bearer caller-token".to_string(),
            )],
            body: b"hello".to_vec(),
            network_policy: sample_policy(),
            credential_injections: vec![],
            response_body_limit: Some(4096),
            timeout_ms: None,
        })
        .expect_err("runtime-supplied sensitive headers should fail before network dispatch");

    assert!(matches!(
        error,
        ironclaw_host_api::RuntimeHttpEgressError::Request { .. }
    ));
    assert!(error.to_string().contains("sensitive_header"));
    assert!(network_recorder.lock().unwrap().is_empty());
}

#[test]
fn host_http_egress_blocks_runtime_supplied_credential_query_before_network() {
    let network = RecordingNetwork::ok(NetworkHttpResponse {
        status: 200,
        headers: vec![],
        body: br#"{"ok":true}"#.to_vec(),
        usage: NetworkUsage {
            request_bytes: 5,
            response_bytes: 11,
            resolved_ip: None,
        },
    });
    let network_recorder = network.requests.clone();
    let service = HostHttpEgressService::new(network, InMemorySecretStore::new());

    let error = service
        .execute(RuntimeHttpEgressRequest {
            runtime: RuntimeKind::Script,
            scope: sample_scope(),
            method: NetworkMethod::Get,
            url: "https://api.example.test/v1/run?api_key=short-manual-key".to_string(),
            headers: vec![],
            body: Vec::new(),
            network_policy: sample_policy(),
            credential_injections: vec![],
            response_body_limit: Some(4096),
            timeout_ms: None,
        })
        .expect_err("runtime-supplied credential query params should fail before network dispatch");

    assert!(matches!(
        error,
        ironclaw_host_api::RuntimeHttpEgressError::Request { .. }
    ));
    assert!(error.to_string().contains("manual_credentials"));
    assert!(network_recorder.lock().unwrap().is_empty());
}

#[test]
fn host_http_egress_blocks_percent_encoded_credential_values_before_network() {
    let network = RecordingNetwork::ok(NetworkHttpResponse {
        status: 200,
        headers: vec![],
        body: br#"{"ok":true}"#.to_vec(),
        usage: NetworkUsage {
            request_bytes: 5,
            response_bytes: 11,
            resolved_ip: None,
        },
    });
    let network_recorder = network.requests.clone();
    let service = HostHttpEgressService::new(network, InMemorySecretStore::new());

    let error = service
        .execute(RuntimeHttpEgressRequest {
            runtime: RuntimeKind::Script,
            scope: sample_scope(),
            method: NetworkMethod::Get,
            url: "https://api.example.test/v1/run?data=AKIA%49OSFODNN7EXAMPLE".to_string(),
            headers: vec![],
            body: Vec::new(),
            network_policy: sample_policy(),
            credential_injections: vec![],
            response_body_limit: Some(4096),
            timeout_ms: None,
        })
        .expect_err("percent-encoded credential values should fail before network dispatch");

    assert!(matches!(
        error,
        ironclaw_host_api::RuntimeHttpEgressError::Request { .. }
    ));
    assert!(!error.to_string().contains("AKIAIOSFODNN7EXAMPLE"));
    assert!(network_recorder.lock().unwrap().is_empty());
}

#[test]
fn host_http_egress_blocks_runtime_supplied_auth_like_headers_before_network() {
    let network = RecordingNetwork::ok(NetworkHttpResponse {
        status: 200,
        headers: vec![],
        body: br#"{"ok":true}"#.to_vec(),
        usage: NetworkUsage {
            request_bytes: 5,
            response_bytes: 11,
            resolved_ip: None,
        },
    });
    let network_recorder = network.requests.clone();
    let service = HostHttpEgressService::new(network, InMemorySecretStore::new());

    let error = service
        .execute(RuntimeHttpEgressRequest {
            runtime: RuntimeKind::Script,
            scope: sample_scope(),
            method: NetworkMethod::Post,
            url: "https://api.example.test/v1/run".to_string(),
            headers: vec![("X-Custom-Auth".to_string(), "short-manual-key".to_string())],
            body: b"hello".to_vec(),
            network_policy: sample_policy(),
            credential_injections: vec![],
            response_body_limit: Some(4096),
            timeout_ms: None,
        })
        .expect_err("runtime-supplied auth-like headers should fail before network dispatch");

    assert!(matches!(
        error,
        ironclaw_host_api::RuntimeHttpEgressError::Request { .. }
    ));
    assert!(error.to_string().contains("manual_credentials"));
    assert!(network_recorder.lock().unwrap().is_empty());
}

#[test]
fn host_http_egress_runs_async_secret_store_futures_with_tokio_context() {
    let network = RecordingNetwork::ok(NetworkHttpResponse {
        status: 200,
        headers: vec![],
        body: br#"{"ok":true}"#.to_vec(),
        usage: NetworkUsage {
            request_bytes: 5,
            response_bytes: 11,
            resolved_ip: None,
        },
    });
    let network_recorder = network.requests.clone();
    let secrets = TokioBackedSecretStore::new();
    let scope = sample_scope();
    let handle = SecretHandle::new("api-token").unwrap();
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(secrets.put(
            scope.clone(),
            handle.clone(),
            SecretMaterial::from("sk-test-secret"),
        ))
        .unwrap();
    let service = HostHttpEgressService::new(network, secrets);

    let response = service
        .execute(RuntimeHttpEgressRequest {
            runtime: RuntimeKind::Script,
            scope,
            method: NetworkMethod::Post,
            url: "https://api.example.test/v1/run".to_string(),
            headers: vec![],
            body: b"hello".to_vec(),
            network_policy: sample_policy(),
            credential_injections: vec![RuntimeCredentialInjection {
                handle,
                target: RuntimeCredentialTarget::Header {
                    name: "authorization".to_string(),
                    prefix: Some("Bearer ".to_string()),
                },
                required: true,
            }],
            response_body_limit: Some(4096),
            timeout_ms: None,
        })
        .expect("host egress should poll async secret stores inside a Tokio context");

    assert_eq!(response.status, 200);
    let requests = network_recorder.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0]
            .headers
            .iter()
            .find(|(name, _)| name == "authorization"),
        Some(&(
            "authorization".to_string(),
            "Bearer sk-test-secret".to_string()
        ))
    );
}

#[test]
fn host_http_egress_maps_network_errors_to_stable_runtime_reasons() {
    let network = RecordingNetwork::err(NetworkHttpError::Transport {
        reason: "connection failed for https://api.example.test/path?token=raw-secret".to_string(),
        request_bytes: 12,
        response_bytes: 0,
    });
    let service = HostHttpEgressService::new(network, InMemorySecretStore::new());

    let error = service
        .execute(RuntimeHttpEgressRequest {
            runtime: RuntimeKind::Script,
            scope: sample_scope(),
            method: NetworkMethod::Post,
            url: "https://api.example.test/v1/run".to_string(),
            headers: vec![],
            body: b"hello".to_vec(),
            network_policy: sample_policy(),
            credential_injections: vec![],
            response_body_limit: Some(4096),
            timeout_ms: None,
        })
        .expect_err("network errors should surface as stable sanitized variants");

    assert!(matches!(
        error,
        ironclaw_host_api::RuntimeHttpEgressError::Network { .. }
    ));
    assert!(error.to_string().contains("transport_failed"));
    assert!(!error.to_string().contains("raw-secret"));
    assert!(!error.to_string().contains("api.example.test/path"));
    assert_eq!(error.request_bytes(), 12);
}

#[derive(Clone)]
struct RecordingNetwork {
    response: Result<NetworkHttpResponse, NetworkHttpError>,
    requests: Arc<Mutex<Vec<NetworkHttpRequest>>>,
}

#[derive(Clone)]
struct UrlEchoNetwork {
    requests: Arc<Mutex<Vec<NetworkHttpRequest>>>,
}

impl UrlEchoNetwork {
    fn new() -> Self {
        Self {
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl NetworkHttpEgress for UrlEchoNetwork {
    fn execute(
        &self,
        request: NetworkHttpRequest,
    ) -> Result<NetworkHttpResponse, NetworkHttpError> {
        self.requests.lock().unwrap().push(request.clone());
        Err(NetworkHttpError::Transport {
            reason: format!("upstream rejected {}", request.url),
            request_bytes: request.body.len() as u64,
            response_bytes: 0,
        })
    }
}

impl RecordingNetwork {
    fn ok(response: NetworkHttpResponse) -> Self {
        Self {
            response: Ok(response),
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn err(error: NetworkHttpError) -> Self {
        Self {
            response: Err(error),
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl NetworkHttpEgress for RecordingNetwork {
    fn execute(
        &self,
        request: NetworkHttpRequest,
    ) -> Result<NetworkHttpResponse, NetworkHttpError> {
        self.requests.lock().unwrap().push(request);
        self.response.clone()
    }
}

struct TokioBackedSecretStore {
    inner: InMemorySecretStore,
}

impl TokioBackedSecretStore {
    fn new() -> Self {
        Self {
            inner: InMemorySecretStore::new(),
        }
    }

    async fn yield_to_tokio() {
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
}

#[async_trait::async_trait]
impl SecretStore for TokioBackedSecretStore {
    async fn put(
        &self,
        scope: ResourceScope,
        handle: SecretHandle,
        material: SecretMaterial,
    ) -> Result<SecretMetadata, SecretStoreError> {
        Self::yield_to_tokio().await;
        self.inner.put(scope, handle, material).await
    }

    async fn metadata(
        &self,
        scope: &ResourceScope,
        handle: &SecretHandle,
    ) -> Result<Option<SecretMetadata>, SecretStoreError> {
        Self::yield_to_tokio().await;
        self.inner.metadata(scope, handle).await
    }

    async fn lease_once(
        &self,
        scope: &ResourceScope,
        handle: &SecretHandle,
    ) -> Result<SecretLease, SecretStoreError> {
        Self::yield_to_tokio().await;
        self.inner.lease_once(scope, handle).await
    }

    async fn consume(
        &self,
        scope: &ResourceScope,
        lease_id: SecretLeaseId,
    ) -> Result<SecretMaterial, SecretStoreError> {
        Self::yield_to_tokio().await;
        self.inner.consume(scope, lease_id).await
    }

    async fn revoke(
        &self,
        scope: &ResourceScope,
        lease_id: SecretLeaseId,
    ) -> Result<SecretLease, SecretStoreError> {
        Self::yield_to_tokio().await;
        self.inner.revoke(scope, lease_id).await
    }

    async fn leases_for_scope(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<SecretLease>, SecretStoreError> {
        Self::yield_to_tokio().await;
        self.inner.leases_for_scope(scope).await
    }
}

fn block_on_test<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(future)
}

fn sample_scope() -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new("tenant1").unwrap(),
        user_id: UserId::new("user1").unwrap(),
        agent_id: None,
        project_id: None,
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    }
}

fn sample_policy() -> NetworkPolicy {
    NetworkPolicy {
        allowed_targets: vec![NetworkTargetPattern {
            scheme: Some(NetworkScheme::Https),
            host_pattern: "api.example.test".to_string(),
            port: None,
        }],
        deny_private_ip_ranges: true,
        max_egress_bytes: Some(4096),
    }
}
