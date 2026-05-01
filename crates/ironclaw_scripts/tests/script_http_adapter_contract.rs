use std::sync::{Arc, Mutex};

use ironclaw_host_api::{
    CapabilityId, InvocationId, NetworkMethod, NetworkPolicy, NetworkScheme, NetworkTargetPattern,
    ProjectId, ResourceScope, RuntimeHttpEgress, RuntimeHttpEgressError, RuntimeHttpEgressRequest,
    RuntimeHttpEgressResponse, RuntimeKind, TenantId, UserId,
};
use ironclaw_scripts::{ScriptHostHttpRequest, ScriptRuntimeHttpAdapter};

#[test]
fn script_host_http_adapter_uses_shared_runtime_egress() {
    let egress = RecordingRuntimeEgress::ok(RuntimeHttpEgressResponse {
        status: 201,
        headers: vec![("content-type".to_string(), "application/json".to_string())],
        body: br#"{"ok":true}"#.to_vec(),
        request_bytes: 7,
        response_bytes: 11,
        redaction_applied: false,
    });
    let adapter = ScriptRuntimeHttpAdapter::new(Arc::new(egress.clone()));
    let scope = sample_scope();

    let response = adapter
        .request(ScriptHostHttpRequest {
            scope: scope.clone(),
            capability_id: sample_capability_id(),
            method: NetworkMethod::Post,
            url: "https://script-api.example.test/run".to_string(),
            headers: vec![("content-type".to_string(), "application/json".to_string())],
            body: br#"{"a":1}"#.to_vec(),
            network_policy: sample_policy(),
            credential_injections: vec![],
            response_body_limit: Some(4096),
            timeout_ms: None,
        })
        .expect("script host-mediated HTTP should succeed");

    assert_eq!(response.status, 201);
    assert_eq!(response.body, br#"{"ok":true}"#);
    assert_eq!(response.request_bytes, 7);
    assert_eq!(response.response_bytes, 11);

    let requests = egress.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].runtime, RuntimeKind::Script);
    assert_eq!(requests[0].scope, scope);
    assert_eq!(requests[0].capability_id, sample_capability_id());
    assert_eq!(requests[0].method, NetworkMethod::Post);
    assert_eq!(requests[0].url, "https://script-api.example.test/run");
    assert_eq!(requests[0].body, br#"{"a":1}"#);
    assert_eq!(requests[0].response_body_limit, Some(4096));
}

#[test]
fn script_host_http_adapter_returns_sanitized_shared_egress_errors() {
    let adapter = ScriptRuntimeHttpAdapter::new(Arc::new(RecordingRuntimeEgress::err(
        RuntimeHttpEgressError::Network {
            reason: "network request denied by policy for sk-test-secret".to_string(),
            request_bytes: 0,
            response_bytes: 0,
        },
    )));

    let error = adapter
        .request(ScriptHostHttpRequest {
            scope: sample_scope(),
            capability_id: sample_capability_id(),
            method: NetworkMethod::Get,
            url: "https://script-api.example.test/run".to_string(),
            headers: vec![],
            body: vec![],
            network_policy: sample_policy(),
            credential_injections: vec![],
            response_body_limit: Some(4096),
            timeout_ms: None,
        })
        .expect_err("network denial should surface as a sanitized adapter error");

    let rendered = error.to_string();
    assert!(rendered.contains("network_error"));
    assert!(!rendered.contains("sk-test-secret"));
    assert!(!rendered.contains("network request denied by policy"));
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

fn sample_capability_id() -> CapabilityId {
    CapabilityId::new("script.http").unwrap()
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
            host_pattern: "script-api.example.test".to_string(),
            port: None,
        }],
        deny_private_ip_ranges: true,
        max_egress_bytes: Some(4096),
    }
}
